//! Активна схема живлення.

use crate::devprop::decode_string;
use crate::model::{format_guid, PowerPlan};
use optimax_core::CoreError;
use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
use windows::Win32::System::Power::{PowerGetActiveScheme, PowerReadFriendlyName};

fn friendly_name(guid: &GUID) -> Option<String> {
    let mut size = 0u32;
    // SAFETY: перший виклик без буфера лише повертає розмір у `size`.
    let status = unsafe { PowerReadFriendlyName(None, Some(guid), None, None, None, &mut size) };
    if status != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: буфер має `size` байт, як вимагає функція.
    let status = unsafe {
        PowerReadFriendlyName(
            None,
            Some(guid),
            None,
            None,
            Some(buf.as_mut_ptr()),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then(|| decode_string(&buf))
}

pub(super) fn read() -> Result<PowerPlan, CoreError> {
    let mut ptr: *mut GUID = std::ptr::null_mut();
    // SAFETY: функція записує вказівник на GUID, виділений через LocalAlloc.
    let status = unsafe { PowerGetActiveScheme(None, &mut ptr) };
    if status != ERROR_SUCCESS || ptr.is_null() {
        return Err(CoreError::Os {
            api: "PowerGetActiveScheme",
            code: status.0,
        });
    }
    // SAFETY: `ptr` ненульовий і вказує на GUID від PowerGetActiveScheme; копіюємо до звільнення.
    let guid = unsafe { *ptr };
    // SAFETY: пам'ять виділена системою через LocalAlloc і звільняється один раз.
    let _ = unsafe { LocalFree(Some(HLOCAL(ptr.cast()))) };
    Ok(PowerPlan {
        guid: format_guid(guid.to_u128()),
        name: friendly_name(&guid),
    })
}
