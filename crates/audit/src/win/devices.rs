//! Відеоадаптери через SetupAPI (драйвер, PCIe) з доповненням від NVML (VRAM, BAR1).

use crate::devprop::{decode_filetime, decode_string, decode_string_list, decode_u32};
use crate::gpu_info::{is_microsoft_virtual, pci_vendor_from_hardware_id, rebar_from_bar1};
use crate::model::{GpuAdapter, GpuVendor, PcieLink, RebarState};
use nvml_wrapper::Nvml;
use optimax_core::{CivilDate, CoreError};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDevicePropertyW, DIGCF_PRESENT, GUID_DEVCLASS_DISPLAY, HDEVINFO, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_Address, DEVPKEY_Device_BusNumber, DEVPKEY_Device_DeviceDesc,
    DEVPKEY_Device_DriverDate, DEVPKEY_Device_DriverVersion, DEVPKEY_Device_HardwareIds,
    DEVPROPTYPE,
};
use windows::Win32::Foundation::DEVPROPKEY;

/// Ключі з `pciprop.h`. У крейті `windows` вони лежать у модулі WiFi, тож визначені тут
/// з тими самими значеннями, щоб не тягнути зайву фічу.
const PCI_FMTID: GUID = GUID::from_u128(0x3ab22e31_8264_4b4e_9af5_a8d2d8e33e62);
const DEVPKEY_PCI_CURRENT_LINK_SPEED: DEVPROPKEY = DEVPROPKEY {
    fmtid: PCI_FMTID,
    pid: 9,
};
const DEVPKEY_PCI_CURRENT_LINK_WIDTH: DEVPROPKEY = DEVPROPKEY {
    fmtid: PCI_FMTID,
    pid: 10,
};
const DEVPKEY_PCI_MAX_LINK_SPEED: DEVPROPKEY = DEVPROPKEY {
    fmtid: PCI_FMTID,
    pid: 11,
};
const DEVPKEY_PCI_MAX_LINK_WIDTH: DEVPROPKEY = DEVPROPKEY {
    fmtid: PCI_FMTID,
    pid: 12,
};

const MIB: u64 = 1024 * 1024;

struct DevInfoSet(HDEVINFO);

impl Drop for DevInfoSet {
    fn drop(&mut self) {
        // SAFETY: набір створено SetupDiGetClassDevsW, знищується один раз.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn property(set: &DevInfoSet, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<Vec<u8>> {
    let mut ty = DEVPROPTYPE::default();
    let mut size = 0u32;
    // SAFETY: запит розміру без буфера; помилка ERROR_INSUFFICIENT_BUFFER очікувана.
    let _ =
        unsafe { SetupDiGetDevicePropertyW(set.0, data, key, &mut ty, None, Some(&mut size), 0) };
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: буфер має розмір, який щойно повернула система.
    unsafe {
        SetupDiGetDevicePropertyW(
            set.0,
            data,
            key,
            &mut ty,
            Some(&mut buf),
            Some(&mut size),
            0,
        )
    }
    .ok()?;
    buf.truncate(size as usize);
    Some(buf)
}

fn u32_prop(set: &DevInfoSet, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<u32> {
    property(set, data, key).as_deref().and_then(decode_u32)
}

/// VRAM і BAR1 з NVML для карти на тій самій PCI-шині/пристрої.
fn nvml_memory(nvml: &Nvml, bus: u32, device: u32) -> Option<(u64, u64)> {
    let count = nvml.device_count().ok()?;
    (0..count).find_map(|i| {
        let dev = nvml.device_by_index(i).ok()?;
        let pci = dev.pci_info().ok()?;
        if pci.bus != bus || pci.device != device {
            return None;
        }
        let vram = dev.memory_info().ok()?.total / MIB;
        let bar1 = dev.bar1_memory_info().ok()?.total / MIB;
        Some((vram, bar1))
    })
}

pub(super) fn read(nvml: Option<&Nvml>) -> Result<Vec<GpuAdapter>, CoreError> {
    // SAFETY: клас пристроїв — валідний GUID, решта параметрів необов'язкові.
    let handle = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_DISPLAY),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT,
        )
    }
    .map_err(|e| CoreError::Os {
        api: "SetupDiGetClassDevsW",
        code: e.code().0 as u32,
    })?;
    let set = DevInfoSet(handle);

    let mut out = Vec::new();
    for index in 0..64 {
        let mut data = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: `data` ініціалізовано з правильним `cbSize`; кінець переліку — помилка.
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut data) }.is_err() {
            break;
        }

        let hardware_ids = property(&set, &data, &DEVPKEY_Device_HardwareIds)
            .map(|b| decode_string_list(&b))
            .unwrap_or_default();
        let Some(vendor_id) = hardware_ids
            .iter()
            .find_map(|id| pci_vendor_from_hardware_id(id))
        else {
            continue; // не PCI-пристрій (віртуальні, ROOT\...)
        };
        let vendor = GpuVendor::from_pci_id(vendor_id);
        if is_microsoft_virtual(vendor) {
            continue;
        }

        let pcie = match (
            u32_prop(&set, &data, &DEVPKEY_PCI_CURRENT_LINK_WIDTH),
            u32_prop(&set, &data, &DEVPKEY_PCI_MAX_LINK_WIDTH),
        ) {
            (Some(current_width), Some(max_width)) if max_width > 0 => Some(PcieLink {
                current_width,
                max_width,
                current_gen: u32_prop(&set, &data, &DEVPKEY_PCI_CURRENT_LINK_SPEED),
                max_gen: u32_prop(&set, &data, &DEVPKEY_PCI_MAX_LINK_SPEED),
            }),
            _ => None,
        };

        let (vram_mib, rebar) = match (vendor, nvml) {
            (GpuVendor::Nvidia, Some(nvml)) => {
                let bus = u32_prop(&set, &data, &DEVPKEY_Device_BusNumber);
                // DEVPKEY_Device_Address для PCI: (пристрій << 16) | функція.
                let device = u32_prop(&set, &data, &DEVPKEY_Device_Address).map(|a| a >> 16);
                match bus.zip(device).and_then(|(b, d)| nvml_memory(nvml, b, d)) {
                    Some((vram, bar1)) => (Some(vram), rebar_from_bar1(bar1, vram)),
                    None => (None, RebarState::Unknown),
                }
            }
            _ => (None, RebarState::Unknown),
        };

        out.push(GpuAdapter {
            name: property(&set, &data, &DEVPKEY_Device_DeviceDesc)
                .map(|b| decode_string(&b))
                .unwrap_or_default(),
            vendor,
            driver_version: property(&set, &data, &DEVPKEY_Device_DriverVersion)
                .map(|b| decode_string(&b)),
            driver_date: property(&set, &data, &DEVPKEY_Device_DriverDate)
                .as_deref()
                .and_then(decode_filetime)
                .map(CivilDate::from_filetime),
            pcie,
            rebar,
            vram_mib,
        });
    }
    Ok(out)
}
