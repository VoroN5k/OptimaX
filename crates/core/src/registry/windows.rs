//! Реалізація [`RegistryApi`] через WinAPI.
#![allow(unsafe_code)]

use super::{RegHive, RegValue, RegistryApi, RegistryWriteApi};
use crate::error::CoreError;
use std::borrow::Cow;
use windows::core::HSTRING;
use windows::core::PWSTR;
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_QUERY_VALUE,
    KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_SAM_FLAGS, REG_VALUE_TYPE,
};

/// Реєстр Windows (64-бітне представлення).
#[derive(Debug, Default, Clone, Copy)]
pub struct WinRegistry;

/// Реєстр Windows, де [`RegHive::CurrentUser`] — гілка **заданого** користувача
/// (`HKEY_USERS\<SID>`), а не процесу. Для служби: у SYSTEM свій `HKCU`, а змінювати треба
/// налаштування користувача, що під'єднався. Гілка завантажена, поки користувач у системі.
#[derive(Debug, Clone)]
pub struct WinUserRegistry {
    sid: String,
}

impl WinUserRegistry {
    /// Для користувача з цим SID (`S-1-5-…`; інше — помилка, щоб не вийти за `HKEY_USERS\<SID>`).
    pub fn new(sid: &str) -> Result<Self, CoreError> {
        if crate::datadir::is_plain_sid(sid) {
            Ok(Self { sid: sid.into() })
        } else {
            Err(CoreError::InvalidData(format!("неочікуваний SID: {sid}")))
        }
    }

    /// SID користувача.
    pub fn sid(&self) -> &str {
        &self.sid
    }
}

/// Корінь і шлях усередині нього.
trait Resolve {
    fn resolve<'p>(&self, hive: RegHive, path: &'p str) -> (HKEY, Cow<'p, str>);
}

impl Resolve for WinRegistry {
    fn resolve<'p>(&self, hive: RegHive, path: &'p str) -> (HKEY, Cow<'p, str>) {
        let root = match hive {
            RegHive::LocalMachine => HKEY_LOCAL_MACHINE,
            RegHive::CurrentUser => HKEY_CURRENT_USER,
        };
        (root, Cow::Borrowed(path))
    }
}

impl Resolve for WinUserRegistry {
    fn resolve<'p>(&self, hive: RegHive, path: &'p str) -> (HKEY, Cow<'p, str>) {
        match hive {
            RegHive::LocalMachine => (HKEY_LOCAL_MACHINE, Cow::Borrowed(path)),
            RegHive::CurrentUser => (
                HKEY_USERS,
                Cow::Owned(format!("{}\\{}", self.sid, path.trim_start_matches('\\'))),
            ),
        }
    }
}

/// Закриває ключ при виході зі скоупу.
struct KeyGuard(HKEY);

impl Drop for KeyGuard {
    fn drop(&mut self) {
        // SAFETY: ключ відкритий RegOpenKeyExW і закривається рівно один раз.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn check(status: WIN32_ERROR, api: &'static str) -> Result<(), CoreError> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(CoreError::Os {
            api,
            code: status.0,
        })
    }
}

/// Найдовше ім'я значення реєстру — 16 383 символи (+ нуль).
const MAX_VALUE_NAME: usize = 16_384;

/// Відкриває **наявний** ключ із заданими правами (ключі не створюються).
fn open_existing(root: HKEY, path: &str, access: REG_SAM_FLAGS) -> Result<KeyGuard, CoreError> {
    let path_w = HSTRING::from(path);
    let mut raw_key = HKEY::default();
    // SAFETY: HSTRING нуль-термінований, `raw_key` — валідний вихідний вказівник.
    let status = unsafe {
        RegOpenKeyExW(
            root,
            &path_w,
            Some(0),
            access | KEY_WOW64_64KEY,
            &mut raw_key,
        )
    };
    check(status, "RegOpenKeyExW")?;
    Ok(KeyGuard(raw_key))
}

fn is_missing(e: &CoreError) -> bool {
    matches!(e, CoreError::Os { code, .. } if *code == ERROR_FILE_NOT_FOUND.0)
}

/// Найдовше ім'я ключа реєстру — 255 символів (+ нуль).
const MAX_KEY_NAME: usize = 256;

fn subkeys_at(root: HKEY, path: &str) -> Result<Vec<String>, CoreError> {
    let key = match open_existing(root, path, KEY_READ) {
        Ok(k) => k,
        Err(e) if is_missing(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names = Vec::new();
    let mut buf = vec![0u16; MAX_KEY_NAME];
    for index in 0u32.. {
        let mut len = u32::try_from(buf.len())
            .map_err(|_| CoreError::InvalidData("буфер завеликий".into()))?;
        // SAFETY: `buf` має `len` u16-елементів; клас і час зміни не запитуємо (None).
        let status = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        check(status, "RegEnumKeyExW")?;
        let end = usize::try_from(len).unwrap_or(0).min(buf.len());
        names.push(String::from_utf16_lossy(&buf[..end]));
    }
    Ok(names)
}

fn names_at(root: HKEY, path: &str) -> Result<Vec<String>, CoreError> {
    let key = match open_existing(root, path, KEY_READ) {
        Ok(k) => k,
        Err(e) if is_missing(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names = Vec::new();
    let mut buf = vec![0u16; MAX_VALUE_NAME];
    for index in 0u32.. {
        let mut len = u32::try_from(buf.len())
            .map_err(|_| CoreError::InvalidData("буфер завеликий".into()))?;
        // SAFETY: `buf` має `len` u16-елементів; дані значення не запитуємо (None).
        let status = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        check(status, "RegEnumValueW")?;
        let end = usize::try_from(len).unwrap_or(0).min(buf.len());
        names.push(String::from_utf16_lossy(&buf[..end]));
    }
    Ok(names)
}

fn read_at(root: HKEY, path: &str, name: &str) -> Result<Option<RegValue>, CoreError> {
    let key = match open_existing(root, path, KEY_READ) {
        Ok(k) => k,
        Err(e) if is_missing(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let name_w = HSTRING::from(name);
    let mut buf = vec![0u8; 256];
    // Значення може змінитися між викликами — кілька спроб із більшим буфером.
    for _ in 0..4 {
        let mut reg_type = REG_VALUE_TYPE::default();
        let mut len = u32::try_from(buf.len())
            .map_err(|_| CoreError::InvalidData("буфер завеликий".into()))?;
        // SAFETY: `buf` має довжину `len` байт, усі вказівники валідні на час виклику.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                &name_w,
                None,
                Some(&mut reg_type),
                Some(buf.as_mut_ptr()),
                Some(&mut len),
            )
        };
        if status == ERROR_MORE_DATA {
            buf.resize(len as usize, 0);
            continue;
        }
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status, "RegQueryValueExW")?;
        buf.truncate(len as usize);
        return RegValue::from_raw(reg_type.0, &buf).map(Some);
    }
    Err(CoreError::InvalidData(format!(
        "значення {name} постійно змінює розмір"
    )))
}

fn write_at(root: HKEY, path: &str, name: &str, value: &RegValue) -> Result<(), CoreError> {
    let key = open_existing(root, path, KEY_SET_VALUE | KEY_QUERY_VALUE)?;
    let (ty, bytes) = value.to_raw();
    let name_w = HSTRING::from(name);
    // SAFETY: ключ відкритий з KEY_SET_VALUE, буфер `bytes` живий на час виклику.
    let status = unsafe { RegSetValueExW(key.0, &name_w, None, REG_VALUE_TYPE(ty), Some(&bytes)) };
    check(status, "RegSetValueExW")
}

fn delete_at(root: HKEY, path: &str, name: &str) -> Result<(), CoreError> {
    let key = match open_existing(root, path, KEY_SET_VALUE) {
        Ok(k) => k,
        // Немає ключа — немає й значення.
        Err(e) if is_missing(&e) => return Ok(()),
        Err(e) => return Err(e),
    };
    let name_w = HSTRING::from(name);
    // SAFETY: ключ відкритий з KEY_SET_VALUE, ім'я нуль-терміноване.
    let status = unsafe { RegDeleteValueW(key.0, &name_w) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status, "RegDeleteValueW")
}

macro_rules! impl_registry {
    ($t:ty) => {
        impl RegistryApi for $t {
            fn value_names(&self, hive: RegHive, path: &str) -> Result<Vec<String>, CoreError> {
                let (root, path) = self.resolve(hive, path);
                names_at(root, &path)
            }

            fn subkey_names(&self, hive: RegHive, path: &str) -> Result<Vec<String>, CoreError> {
                let (root, path) = self.resolve(hive, path);
                subkeys_at(root, &path)
            }

            fn read_value(
                &self,
                hive: RegHive,
                path: &str,
                name: &str,
            ) -> Result<Option<RegValue>, CoreError> {
                let (root, path) = self.resolve(hive, path);
                read_at(root, &path, name)
            }
        }

        impl RegistryWriteApi for $t {
            fn write_value(
                &self,
                hive: RegHive,
                path: &str,
                name: &str,
                value: &RegValue,
            ) -> Result<(), CoreError> {
                let (root, path) = self.resolve(hive, path);
                write_at(root, &path, name, value)
            }

            fn delete_value(&self, hive: RegHive, path: &str, name: &str) -> Result<(), CoreError> {
                let (root, path) = self.resolve(hive, path);
                delete_at(root, &path, name)
            }
        }
    };
}

impl_registry!(WinRegistry);
impl_registry!(WinUserRegistry);

#[cfg(test)]
mod tests {
    use super::*;

    /// Справжній реєстр, лише читання: ключ версії Windows є на кожній системі.
    #[test]
    fn value_names_lists_real_key() {
        let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let names = WinRegistry.value_names(RegHive::LocalMachine, key).unwrap();
        assert!(names.iter().any(|n| n == "ProductName"), "{names:?}");
        assert!(WinRegistry
            .value_names(RegHive::LocalMachine, r"SOFTWARE\OptimaX-no-such-key")
            .unwrap()
            .is_empty());
    }

    /// Підключі справжнього ключа: `Uninstall` є в `CurrentVersion` на кожній системі.
    #[test]
    fn subkey_names_lists_real_key() {
        let key = r"SOFTWARE\Microsoft\Windows\CurrentVersion";
        let names = WinRegistry
            .subkey_names(RegHive::LocalMachine, key)
            .unwrap();
        assert!(names.iter().any(|n| n == "Uninstall"), "{names:?}");
        assert!(WinRegistry
            .subkey_names(RegHive::LocalMachine, r"SOFTWARE\OptimaX-no-such-key")
            .unwrap()
            .is_empty());
    }

    /// Гілка поточного користувача через `HKEY_USERS\<SID>` — та сама, що й `HKCU`.
    #[test]
    fn user_registry_reads_the_same_hive_as_hkcu() {
        let sid = crate::datadir::current_user_sid().unwrap();
        let user = WinUserRegistry::new(&sid).unwrap();
        let key = r"Control Panel\Desktop";
        let mut a = WinRegistry.value_names(RegHive::CurrentUser, key).unwrap();
        let mut b = user.value_names(RegHive::CurrentUser, key).unwrap();
        a.sort();
        b.sort();
        assert!(!a.is_empty());
        assert_eq!(a, b);
        assert_eq!(
            WinRegistry
                .read_value(RegHive::CurrentUser, key, "WheelScrollLines")
                .unwrap(),
            user.read_value(RegHive::CurrentUser, key, "WheelScrollLines")
                .unwrap()
        );
        assert!(WinUserRegistry::new(r"S-1-5-21\..\x").is_err());
        assert!(WinUserRegistry::new("..").is_err());
    }
}
