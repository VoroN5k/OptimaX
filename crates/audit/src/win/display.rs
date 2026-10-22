//! Монітори та їхні режими через GDI.

use crate::devprop::wide_array_to_string;
use crate::model::DisplayInfo;
use optimax_core::CoreError;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW,
    DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICE_PRIMARY_DEVICE, ENUM_CURRENT_SETTINGS,
    ENUM_DISPLAY_SETTINGS_MODE,
};

/// Запобіжник від нескінченного переліку режимів у кривих драйверах.
const MAX_MODES: u32 = 10_000;

fn display_device() -> DISPLAY_DEVICEW {
    DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    }
}

fn devmode() -> DEVMODEW {
    DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    }
}

fn monitor_name(adapter: &HSTRING) -> Option<String> {
    let mut mon = display_device();
    // SAFETY: `adapter` — нуль-термінований рядок, `mon` ініціалізовано з правильним `cb`.
    let ok = unsafe { EnumDisplayDevicesW(adapter, 0, &mut mon, 0) }.as_bool();
    ok.then(|| wide_array_to_string(&mon.DeviceString))
        .filter(|s| !s.is_empty())
}

pub(super) fn read() -> Result<Vec<DisplayInfo>, CoreError> {
    let mut out = Vec::new();
    for index in 0..64 {
        let mut dd = display_device();
        // SAFETY: `dd` ініціалізовано з правильним `cb`; null-ім'я означає перелік адаптерів.
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut dd, 0) }.as_bool() {
            break;
        }
        if dd.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != DISPLAY_DEVICE_ATTACHED_TO_DESKTOP
        {
            continue;
        }
        let device_name = wide_array_to_string(&dd.DeviceName);
        let name_w = HSTRING::from(device_name.as_str());

        let mut current = devmode();
        // SAFETY: `name_w` нуль-термінований, `current` має коректний `dmSize`.
        if !unsafe { EnumDisplaySettingsW(&name_w, ENUM_CURRENT_SETTINGS, &mut current) }.as_bool()
        {
            continue;
        }

        let mut max_hz = current.dmDisplayFrequency;
        let mut mode = devmode();
        for i in 0..MAX_MODES {
            // SAFETY: як вище; індекс режиму — звичайне число, кінець переліку повертає FALSE.
            if !unsafe { EnumDisplaySettingsW(&name_w, ENUM_DISPLAY_SETTINGS_MODE(i), &mut mode) }
                .as_bool()
            {
                break;
            }
            if mode.dmPelsWidth == current.dmPelsWidth && mode.dmPelsHeight == current.dmPelsHeight
            {
                max_hz = max_hz.max(mode.dmDisplayFrequency);
            }
        }

        out.push(DisplayInfo {
            monitor_name: monitor_name(&name_w)
                .unwrap_or_else(|| wide_array_to_string(&dd.DeviceString)),
            device_name,
            primary: dd.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE == DISPLAY_DEVICE_PRIMARY_DEVICE,
            width: current.dmPelsWidth,
            height: current.dmPelsHeight,
            current_hz: current.dmDisplayFrequency,
            max_hz_at_current_res: max_hz,
        });
    }
    Ok(out)
}
