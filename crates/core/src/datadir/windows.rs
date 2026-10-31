//! Реалізація [`super`] на Windows: відомі теки ([`crate::sysdirs`]), SID поточного
//! користувача, створення тек із дескриптором безпеки й перевірка власника за дескриптором файлу.
#![allow(unsafe_code)]

use super::{untrusted_name, APP_DIR};
use crate::safefs::{self, SafeFsError};
use crate::sysdirs::{self, Folder};
use crate::CoreError;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, ERROR_ALREADY_EXISTS, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetTokenInformation, IsWellKnownSid, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::CreateDirectoryW;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Спільна тека програми: SYSTEM і Адміністратори — повний доступ, більше нікого.
const SDDL_APP: &str = "O:BAD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// Тека користувача: як вище, плюс сам користувач — читання й обхід (0x1200a9).
fn sddl_user(sid: &str) -> String {
    format!("O:BAD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;{sid})")
}

fn os_err(api: &'static str, e: &windows::core::Error) -> CoreError {
    CoreError::Os {
        api,
        code: (e.code().0 & 0xFFFF) as u32,
    }
}

/// Звільняє пам'ять, виділену LocalAlloc.
struct Local(*mut std::ffi::c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: вказівник отримано від API, що виділяє через LocalAlloc; звільняється раз.
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0))) };
        }
    }
}

pub(super) fn program_data() -> Result<PathBuf, CoreError> {
    sysdirs::known(Folder::ProgramData).ok_or(CoreError::NotFound("ProgramData".into()))
}

pub(super) fn local_app_data() -> Option<PathBuf> {
    sysdirs::known(Folder::LocalAppData)
}

/// SID користувача процесу у вигляді `S-1-5-21-…`.
fn user_sid() -> Result<String, CoreError> {
    let mut token = HANDLE::default();
    // SAFETY: псевдодескриптор поточного процесу; `token` — вихідний параметр.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| os_err("OpenProcessToken", &e))?;
    let result = (|| {
        let mut len = 0u32;
        // SAFETY: запит розміру буфера; помилка ERROR_INSUFFICIENT_BUFFER очікувана.
        let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut len) };
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        // SAFETY: буфер вирівняний і має щонайменше `len` байтів.
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr().cast()),
                len,
                &mut len,
            )
        }
        .map_err(|e| os_err("GetTokenInformation", &e))?;
        // SAFETY: система заповнила буфер структурою TOKEN_USER.
        let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut s = PWSTR::null();
        // SAFETY: SID вказує всередину живого `buf`; рядок звільняється через `Local`.
        unsafe { ConvertSidToStringSidW(sid, &mut s) }
            .map_err(|e| os_err("ConvertSidToStringSidW", &e))?;
        let _free = Local(s.0.cast());
        // SAFETY: нуль-термінований рядок від системи.
        unsafe { s.to_string() }.map_err(|_| CoreError::InvalidData("SID".into()))
    })();
    // SAFETY: дескриптор отримано від OpenProcessToken, закривається один раз.
    let _ = unsafe { CloseHandle(token) };
    let sid = result?;
    // Ім'я теки — лише SID: жодних роздільників чи `..`.
    if sid.starts_with("S-")
        && sid
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
    {
        Ok(sid)
    } else {
        Err(CoreError::InvalidData(format!("неочікуваний SID: {sid}")))
    }
}

/// Створює теку з дескриптором безпеки. `Ok(false)` — тека вже існує.
fn create_with(path: &Path, sddl: &str) -> Result<bool, CoreError> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: SDDL — валідний рядок, `sd` — вихідний параметр (звільняється через `Local`).
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(|e| os_err("ConvertStringSecurityDescriptorToSecurityDescriptorW", &e))?;
    let _free = Local(sd.0);
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    // SAFETY: шлях — HSTRING, `sa` і дескриптор живі до кінця виклику.
    match unsafe { CreateDirectoryW(&HSTRING::from(path.as_os_str()), Some(&sa)) } {
        Ok(()) => Ok(true),
        Err(e) if e.code() == ERROR_ALREADY_EXISTS.to_hresult() => Ok(false),
        Err(e) => Err(os_err("CreateDirectoryW", &e)),
    }
}

/// Тека існує, це справжній каталог саме за цим шляхом і її власник — Адміністратори чи SYSTEM.
fn is_trusted(path: &Path) -> Result<bool, CoreError> {
    let dir = match safefs::open_dir_for_security(path) {
        Ok(d) => d,
        Err(
            SafeFsError::Redirected { .. } | SafeFsError::Reparse(_) | SafeFsError::WrongKind(_),
        ) => return Ok(false),
        Err(SafeFsError::Io(e)) => {
            return Err(CoreError::InvalidData(format!("{}: {e}", path.display())))
        }
    };
    let mut owner = PSID::default();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: дескриптор теки відкрито з READ_CONTROL; вихідні параметри — локальні змінні,
    // `sd` звільняється через `Local`, `owner` вказує всередину `sd`.
    let status = unsafe {
        GetSecurityInfo(
            HANDLE(dir.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut sd),
        )
    };
    let _free = Local(sd.0);
    if status.is_err() {
        return Err(CoreError::Os {
            api: "GetSecurityInfo",
            code: status.0,
        });
    }
    // SAFETY: `owner` — валідний SID усередині живого `sd`.
    let trusted = unsafe {
        IsWellKnownSid(owner, WinBuiltinAdministratorsSid).as_bool()
            || IsWellKnownSid(owner, WinLocalSystemSid).as_bool()
    };
    Ok(trusted)
}

/// Гарантує захищену теку `path`: створює, або перевіряє наявну, або відсуває підкинуту.
fn ensure(path: &Path, sddl: &str) -> Result<(), CoreError> {
    if create_with(path, sddl)? || is_trusted(path)? {
        return Ok(());
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(CoreError::InvalidData(path.display().to_string()));
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let aside = parent.join(untrusted_name(&name.to_string_lossy(), now));
    tracing::warn!(
        "тека {} створена не адміністратором (або є посиланням) — її вміст не читаємо, відсуваємо в {}",
        path.display(),
        aside.display()
    );
    // Перейменовується сам об'єкт за шляхом (і junction — як посилання, без переходу в ціль).
    std::fs::rename(path, &aside).map_err(|e| {
        CoreError::InvalidData(format!(
            "{}: тека даних має небезпечні права, і її не вдалося відсунути ({e}); видаліть її вручну",
            path.display()
        ))
    })?;
    if create_with(path, sddl)? && is_trusted(path)? {
        Ok(())
    } else {
        Err(CoreError::InvalidData(format!(
            "{}: не вдалося створити захищену теку даних",
            path.display()
        )))
    }
}

pub(super) fn shared_dir(base: &Path, name: &str) -> Result<PathBuf, CoreError> {
    let app = base.join(APP_DIR);
    ensure(&app, SDDL_APP)?;
    let dir = app.join(name);
    ensure(&dir, SDDL_APP)?;
    Ok(dir)
}

pub(super) fn secure_dir_on(base: &Path) -> Result<PathBuf, CoreError> {
    let app = base.join(APP_DIR);
    ensure(&app, SDDL_APP)?;
    let sid = user_sid()?;
    let user = app.join(&sid);
    ensure(&user, &sddl_user(&sid))?;
    Ok(user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_sid_is_plain() {
        let sid = user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
    }

    /// Потребує прав адміністратора (власник «Адміністратори» при створенні); без них —
    /// пропускається з повідомленням. Працює в окремій тимчасовій теці.
    #[test]
    fn planted_dir_and_junction_are_moved_aside() {
        let base = std::env::temp_dir().join(format!("optimax-datadir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let app = base.join(APP_DIR);
        // Підкинута тека: власник — сам користувач, повний доступ — усім.
        let sid = user_sid().unwrap();
        create_with(&app, &format!("O:{sid}D:(A;OICI;FA;;;WD)")).unwrap();
        std::fs::write(app.join("journal.json"), "forged").unwrap();
        assert!(!is_trusted(&app).unwrap());
        match secure_dir_on(&base) {
            // ERROR_INVALID_OWNER: процес без прав адміністратора не може призначити власника BA.
            Err(CoreError::Os { code: 1307, .. }) => {
                eprintln!("потрібні права адміністратора — тест пропущено");
                let _ = std::fs::remove_dir_all(&base);
                return;
            }
            r => {
                r.unwrap();
            }
        }
        assert!(is_trusted(&app).unwrap());
        assert!(
            !app.join("journal.json").exists(),
            "підкинутий вміст не читається"
        );
        let aside = |b: &Path| {
            std::fs::read_dir(b)
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("OptimaX.untrusted-")
                })
                .count()
        };
        assert_eq!(aside(&base), 1);

        // Junction замість теки: відсувається саме посилання, ціль не змінюється.
        let base2 = base.join("b2");
        let target = base.join("target");
        std::fs::create_dir_all(&base2).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(base2.join(APP_DIR))
            .arg(&target)
            .output()
            .is_ok_and(|o| o.status.success());
        if ok {
            secure_dir_on(&base2).unwrap();
            assert!(is_trusted(&base2.join(APP_DIR)).unwrap());
            assert_eq!(aside(&base2), 1);
            assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn known_folders_resolve() {
        assert!(program_data().unwrap().is_absolute());
        assert!(local_app_data().unwrap().is_absolute());
    }
}
