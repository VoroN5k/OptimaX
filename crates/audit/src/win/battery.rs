//! Читання стану батареї: `GetSystemPowerStatus` + IOCTL через SetupAPI.
#![allow(unsafe_code)]

use crate::model::{BatteryInfo, PowerSource};
use optimax_core::CoreError;
use windows::core::GUID;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA, SP_DEVINFO_DATA,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Power::GetSystemPowerStatus;
use windows::Win32::System::IO::DeviceIoControl;

/// Інтерфейс батареї (GUID_DEVINTERFACE_BATTERY).
const GUID_DEVINTERFACE_BATTERY: GUID = GUID {
    data1: 0x7263_1E54,
    data2: 0x78A4,
    data3: 0x11D0,
    data4: [0xBC, 0xF7, 0x00, 0xAA, 0x00, 0xB7, 0xB3, 0x2A],
};

const IOCTL_BATTERY_QUERY_TAG: u32 = 0x0029_4040;
const IOCTL_BATTERY_QUERY_INFORMATION: u32 = 0x0029_4044;
const IOCTL_BATTERY_QUERY_STATUS: u32 = 0x0029_404C;
const BATTERY_INFORMATION_LEVEL: u32 = 0; // BatteryInformation

#[repr(C)]
struct RawQueryInformation {
    battery_tag: u32,
    information_level: u32,
    at_rate: i32,
}

#[repr(C)]
struct RawBatteryInformation {
    capabilities: u32,
    technology: u8,
    _reserved: [u8; 3],
    chemistry: [u8; 4],
    design_capacity: u32,
    full_charged_capacity: u32,
    default_alert1: u32,
    default_alert2: u32,
    critical_bias: u32,
    cycle_count: u32,
}

#[repr(C)]
struct RawWaitStatus {
    battery_tag: u32,
    timeout: u32,
    power_state: u32,
    low_capacity: u32,
    high_capacity: u32,
}

#[repr(C)]
struct RawBatteryStatus {
    power_state: u32,
    capacity: u32,
    voltage: u32,
    rate: i32,
}

const BATTERY_CAPACITY_RELATIVE: u32 = 0x4000_0000;
const BATTERY_STATUS_UNKNOWN_RATE: i32 = i32::MIN; // 0x80000000

fn ioctl<In, Out>(handle: HANDLE, code: u32, input: &In, output: &mut Out) -> bool {
    let mut returned = 0u32;
    // SAFETY: handle валідний; буфери вказують на коректні структури заданого розміру.
    unsafe {
        DeviceIoControl(
            handle,
            code,
            Some(input as *const In as *const _),
            u32::try_from(std::mem::size_of::<In>()).unwrap_or(0),
            Some(output as *mut Out as *mut _),
            u32::try_from(std::mem::size_of::<Out>()).unwrap_or(0),
            Some(&mut returned),
            None,
        )
        .is_ok()
    }
}

/// Читає `BatteryInformation` і `BatteryStatus` для одного пристрою батареї.
fn read_battery_device(handle: HANDLE) -> Option<(RawBatteryInformation, RawBatteryStatus)> {
    // 1. Отримуємо тег.
    let wait_tag = 0u32;
    let mut tag = 0u32;
    if !ioctl(handle, IOCTL_BATTERY_QUERY_TAG, &wait_tag, &mut tag) || tag == 0 {
        return None;
    }

    // 2. Запитуємо інформацію.
    let query = RawQueryInformation {
        battery_tag: tag,
        information_level: BATTERY_INFORMATION_LEVEL,
        at_rate: 0,
    };
    let mut info = std::mem::MaybeUninit::<RawBatteryInformation>::zeroed();
    // SAFETY: info ініціалізується нулями, тому будь-яке часткове заповнення безпечне.
    let info_ref = unsafe { &mut *info.as_mut_ptr() };
    if !ioctl(handle, IOCTL_BATTERY_QUERY_INFORMATION, &query, info_ref) {
        return None;
    }
    // SAFETY: ioctl заповнив структуру.
    let info = unsafe { info.assume_init() };

    // Відносна ємність — непридатна для розрахунку зносу.
    if info.capabilities & BATTERY_CAPACITY_RELATIVE != 0 {
        return None;
    }

    // 3. Запитуємо статус.
    let wait_status = RawWaitStatus {
        battery_tag: tag,
        timeout: 0,
        power_state: 0,
        low_capacity: 0,
        high_capacity: u32::MAX,
    };
    let mut status = std::mem::MaybeUninit::<RawBatteryStatus>::zeroed();
    // SAFETY: status ініціалізується нулями.
    let status_ref = unsafe { &mut *status.as_mut_ptr() };
    if !ioctl(handle, IOCTL_BATTERY_QUERY_STATUS, &wait_status, status_ref) {
        return None;
    }
    // SAFETY: ioctl заповнив структуру.
    Some((info, unsafe { status.assume_init() }))
}

/// Читає стан батареї. Повертає `Ok(None)` якщо батареї немає.
pub fn read() -> Result<Option<BatteryInfo>, CoreError> {
    // Базовий стан — AC/DC і відсоток заряду.
    let power_status = {
        let mut s = windows::Win32::System::Power::SYSTEM_POWER_STATUS::default();
        // SAFETY: s — валідна структура.
        let ok = unsafe { GetSystemPowerStatus(&mut s) };
        if ok.is_err() {
            return Err(CoreError::Os {
                api: "GetSystemPowerStatus",
                code: 0,
            });
        }
        s
    };

    // BatteryFlag 128 = системна батарея відсутня.
    if power_status.BatteryFlag & 128 != 0 || power_status.ACLineStatus == 255 {
        return Ok(None);
    }

    let source = match power_status.ACLineStatus {
        1 => PowerSource::Ac,
        0 => PowerSource::Dc,
        _ => PowerSource::Unknown,
    };
    let charge_pct = if power_status.BatteryLifePercent <= 100 {
        Some(power_status.BatteryLifePercent)
    } else {
        None
    };

    // Додаткові дані через SetupAPI + IOCTL.
    let (design_mwh, full_charge_mwh, cycle_count, rate_mw) = read_via_ioctl();

    Ok(Some(BatteryInfo {
        source,
        charge_pct,
        design_mwh,
        full_charge_mwh,
        cycle_count,
        rate_mw,
    }))
}

/// Читає детальні дані батареї через SetupAPI та IOCTL.
/// Повертає `(design_mwh, full_charge_mwh, cycle_count, rate_mw)`.
fn read_via_ioctl() -> (Option<u32>, Option<u32>, Option<u32>, Option<i32>) {
    // SAFETY: виклик SetupDi з коректними параметрами; звільнення гарантовано через guard.
    let hdev = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_BATTERY),
            None,
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    let hdev = match hdev {
        Ok(h) if !h.is_invalid() => h,
        _ => return (None, None, None, None),
    };

    let mut design_mwh = None;
    let mut full_charge_mwh = None;
    let mut cycle_count = None;
    let mut rate_mw = None;

    let mut idx = 0u32;
    loop {
        let mut iface = SP_DEVICE_INTERFACE_DATA {
            cbSize: u32::try_from(std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>()).unwrap_or(28),
            ..Default::default()
        };
        // SAFETY: hdev валідний; iface ініціалізований.
        let ok = unsafe {
            SetupDiEnumDeviceInterfaces(hdev, None, &GUID_DEVINTERFACE_BATTERY, idx, &mut iface)
        };
        if ok.is_err() {
            break;
        }
        idx += 1;

        // Визначаємо потрібний розмір буфера.
        let mut required = 0u32;
        let mut devinfo = SP_DEVINFO_DATA {
            cbSize: u32::try_from(std::mem::size_of::<SP_DEVINFO_DATA>()).unwrap_or(32),
            ..Default::default()
        };
        // SAFETY: виклик без буфера лише повертає потрібний розмір.
        let _ = unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                hdev,
                &iface,
                None,
                0,
                Some(&mut required),
                Some(&mut devinfo),
            )
        };
        if required == 0 || required > 4096 {
            continue;
        }

        // Виділяємо буфер для шляху пристрою.
        let mut buf = vec![0u8; required as usize];
        // SP_DEVICE_INTERFACE_DETAIL_DATA_W: перше поле DWORD cbSize, решта — рядок UTF-16.
        // cbSize = розмір заголовку структури (не всього буфера): 8 на x64, 6 на x86.
        let cb_size: u32 = if cfg!(target_pointer_width = "64") {
            8
        } else {
            6
        };
        buf[..4].copy_from_slice(&cb_size.to_ne_bytes());

        // SAFETY: buf достатнього розміру; devinfo ініціалізований.
        let ok = unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                hdev,
                &iface,
                Some(buf.as_mut_ptr().cast()),
                required,
                None,
                Some(&mut devinfo),
            )
        };
        if ok.is_err() {
            continue;
        }

        // Рядок UTF-16 починається зі зміщення 4 (після cbSize DWORD).
        let path_u16: Vec<u16> = buf[4..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        if path_u16.is_empty() {
            continue;
        }
        let path: Vec<u16> = path_u16.iter().copied().chain(std::iter::once(0)).collect();

        // SAFETY: path — коректний нуль-термінований UTF-16.
        let hbatt = unsafe {
            CreateFileW(
                windows::core::PCWSTR(path.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };
        let hbatt = match hbatt {
            Ok(h) if !h.is_invalid() => h,
            _ => continue,
        };

        if let Some((info, status)) = read_battery_device(hbatt) {
            // Зберігаємо найбільшу ємність (якщо кілька батарей — сумарна).
            let d = if info.design_capacity != u32::MAX {
                Some(info.design_capacity)
            } else {
                None
            };
            let f = if info.full_charged_capacity != u32::MAX {
                Some(info.full_charged_capacity)
            } else {
                None
            };
            let cyc = if info.cycle_count != u32::MAX {
                Some(info.cycle_count)
            } else {
                None
            };
            let r = if status.rate != BATTERY_STATUS_UNKNOWN_RATE {
                Some(status.rate)
            } else {
                None
            };
            // Перша батарея виграє; якщо кілька — пропускаємо решту.
            design_mwh = design_mwh.or(d);
            full_charge_mwh = full_charge_mwh.or(f);
            cycle_count = cycle_count.or(cyc);
            rate_mw = rate_mw.or(r);
        }

        // SAFETY: hbatt — валідний handle.
        let _ = unsafe { CloseHandle(hbatt) };
        // Зупиняємось після першої знайденої батареї.
        if design_mwh.is_some() || full_charge_mwh.is_some() {
            break;
        }
    }

    // SAFETY: hdev — валідний handle.
    let _ = unsafe { SetupDiDestroyDeviceInfoList(hdev) };
    (design_mwh, full_charge_mwh, cycle_count, rate_mw)
}
