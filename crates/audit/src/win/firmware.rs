//! Читання таблиці SMBIOS.

use crate::model::FirmwareInfo;
use crate::smbios::parse_raw_smbios;
use optimax_core::CoreError;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::SystemInformation::{GetSystemFirmwareTable, RSMB};

pub(super) fn read() -> Result<FirmwareInfo, CoreError> {
    // SAFETY: буфер не передається — функція лише повертає потрібний розмір.
    let size = unsafe { GetSystemFirmwareTable(RSMB, 0, None) };
    if size == 0 {
        // SAFETY: читає код помилки поточного потоку.
        let code = unsafe { GetLastError() }.0;
        return Err(CoreError::Os {
            api: "GetSystemFirmwareTable",
            code,
        });
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: буфер має саме той розмір, який повернув попередній виклик.
    let written = unsafe { GetSystemFirmwareTable(RSMB, 0, Some(&mut buf)) };
    if written == 0 || written as usize > buf.len() {
        // SAFETY: читає код помилки поточного потоку.
        let code = unsafe { GetLastError() }.0;
        return Err(CoreError::Os {
            api: "GetSystemFirmwareTable",
            code,
        });
    }
    buf.truncate(written as usize);
    parse_raw_smbios(&buf)
}
