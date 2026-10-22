//! Обмеження прав на каталог із вбудованим бінарником: власник — Адміністратори,
//! повний доступ — SYSTEM і Адміністратори, звичайні користувачі — лише читання й запуск.
#![allow(unsafe_code)]

use optimax_core::CoreError;
use std::path::Path;
use windows::core::HSTRING;
use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, ACL, DACL_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};

/// O:BA — власник «Адміністратори»; D:PAI — захищений DACL без успадкування від батька;
/// SY/BA — повний доступ; BU — читання й виконання (0x1200a9) для вкладених файлів і тек.
const SDDL: &str = "O:BAD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";

/// Звільняє дескриптор безпеки, виділений системою.
struct SdGuard(PSECURITY_DESCRIPTOR);

impl Drop for SdGuard {
    fn drop(&mut self) {
        // SAFETY: дескриптор виділено ConvertStringSecurityDescriptor… через LocalAlloc.
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0 .0))) };
    }
}

fn os_err(api: &'static str, e: &windows::core::Error) -> CoreError {
    CoreError::Os {
        api,
        code: e.code().0 as u32,
    }
}

/// Встановлює права. Потребує прав адміністратора (зміна власника на BA).
pub(crate) fn restrict_to_admins(dir: &Path) -> Result<(), CoreError> {
    let mut raw = PSECURITY_DESCRIPTOR::default();
    // SAFETY: SDDL — валідний нуль-термінований рядок, `raw` — вихідний параметр.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(SDDL),
            SDDL_REVISION_1,
            &mut raw,
            None,
        )
    }
    .map_err(|e| os_err("ConvertStringSecurityDescriptorToSecurityDescriptorW", &e))?;
    let sd = SdGuard(raw);

    let mut present = windows::core::BOOL::default();
    let mut defaulted = windows::core::BOOL::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: `sd` — валідний дескриптор, вказівники ведуть на локальні змінні.
    unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted) }
        .map_err(|e| os_err("GetSecurityDescriptorDacl", &e))?;
    let mut owner = PSID::default();
    // SAFETY: як вище.
    unsafe { GetSecurityDescriptorOwner(sd.0, &mut owner, &mut defaulted) }
        .map_err(|e| os_err("GetSecurityDescriptorOwner", &e))?;

    // SAFETY: шлях нуль-термінований; `owner` і `dacl` вказують всередину `sd`, який живий до кінця.
    let status = unsafe {
        SetNamedSecurityInfoW(
            &HSTRING::from(dir.as_os_str()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            Some(owner),
            None,
            Some(dacl.cast_const()),
            None,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(CoreError::Os {
            api: "SetNamedSecurityInfoW",
            code: status.0,
        })
    }
}
