//! Монітори та їхні режими через GDI.

use crate::devprop::wide_array_to_string;
use crate::model::DisplayInfo;
use optimax_core::CoreError;
use std::collections::HashMap;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
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

/// Назви моніторів з EDID (як у Параметрах Windows) за GDI-іменем виходу `\\.\DISPLAYn`.
/// Порожня мапа, якщо API недоступне — тоді лишається назва з EnumDisplayDevices.
fn friendly_names() -> HashMap<String, String> {
    let mut names = HashMap::new();
    // Топологія може змінитися між викликами — кілька спроб.
    for _ in 0..3 {
        let (mut n_paths, mut n_modes) = (0u32, 0u32);
        // SAFETY: обидва вказівники ведуть на локальні змінні.
        if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes) }
            != ERROR_SUCCESS
        {
            return names;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
        // SAFETY: масиви мають рівно `n_paths` і `n_modes` елементів.
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut n_paths,
                paths.as_mut_ptr(),
                &mut n_modes,
                modes.as_mut_ptr(),
                None,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != ERROR_SUCCESS {
            return names;
        }
        paths.truncate(n_paths as usize);

        for path in &paths {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            // SAFETY: вказівник на всю структуру (заголовок — її перше поле, repr(C)),
            // `header.size` дорівнює її розміру, тож система пише в межах структури.
            let ok = unsafe {
                DisplayConfigGetDeviceInfo(
                    (&mut source as *mut DISPLAYCONFIG_SOURCE_DEVICE_NAME).cast(),
                ) == 0
                    && DisplayConfigGetDeviceInfo(
                        (&mut target as *mut DISPLAYCONFIG_TARGET_DEVICE_NAME).cast(),
                    ) == 0
            };
            if !ok {
                continue;
            }
            let friendly = wide_array_to_string(&target.monitorFriendlyDeviceName);
            if !friendly.is_empty() {
                names.insert(wide_array_to_string(&source.viewGdiDeviceName), friendly);
            }
        }
        return names;
    }
    names
}

fn monitor_name(adapter: &HSTRING) -> Option<String> {
    let mut mon = display_device();
    // SAFETY: `adapter` — нуль-термінований рядок, `mon` ініціалізовано з правильним `cb`.
    let ok = unsafe { EnumDisplayDevicesW(adapter, 0, &mut mon, 0) }.as_bool();
    ok.then(|| wide_array_to_string(&mon.DeviceString))
        .filter(|s| !s.is_empty())
}

pub(super) fn read() -> Result<Vec<DisplayInfo>, CoreError> {
    let friendly = friendly_names();
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
            monitor_name: friendly
                .get(&device_name)
                .cloned()
                .or_else(|| monitor_name(&name_w))
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
