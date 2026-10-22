//! Реалізація [`RegistryApi`] через WinAPI.
#![allow(unsafe_code)]

use super::{RegHive, RegValue, RegistryApi, RegistryWriteApi};
use crate::error::CoreError;
use windows::core::HSTRING;
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE,
    KEY_WOW64_64KEY, REG_SAM_FLAGS, REG_VALUE_TYPE,
};

/// Реєстр Windows (64-бітне представлення).
#[derive(Debug, Default, Clone, Copy)]
pub struct WinRegistry;

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

impl RegistryApi for WinRegistry {
    fn read_value(
        &self,
        hive: RegHive,
        path: &str,
        name: &str,
    ) -> Result<Option<RegValue>, CoreError> {
        let root = match hive {
            RegHive::LocalMachine => HKEY_LOCAL_MACHINE,
            RegHive::CurrentUser => HKEY_CURRENT_USER,
        };
        let path_w = HSTRING::from(path);
        let mut raw_key = HKEY::default();
        // SAFETY: HSTRING гарантує нуль-термінований рядок, `raw_key` — валідний вихідний вказівник.
        let status = unsafe {
            RegOpenKeyExW(
                root,
                &path_w,
                Some(0),
                KEY_READ | KEY_WOW64_64KEY,
                &mut raw_key,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status, "RegOpenKeyExW")?;
        let key = KeyGuard(raw_key);

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
}

fn root(hive: RegHive) -> HKEY {
    match hive {
        RegHive::LocalMachine => HKEY_LOCAL_MACHINE,
        RegHive::CurrentUser => HKEY_CURRENT_USER,
    }
}

/// Відкриває **наявний** ключ із заданими правами (ключі не створюються).
fn open_existing(hive: RegHive, path: &str, access: REG_SAM_FLAGS) -> Result<KeyGuard, CoreError> {
    let path_w = HSTRING::from(path);
    let mut raw_key = HKEY::default();
    // SAFETY: HSTRING нуль-термінований, `raw_key` — валідний вихідний вказівник.
    let status = unsafe {
        RegOpenKeyExW(
            root(hive),
            &path_w,
            Some(0),
            access | KEY_WOW64_64KEY,
            &mut raw_key,
        )
    };
    check(status, "RegOpenKeyExW")?;
    Ok(KeyGuard(raw_key))
}

impl RegistryWriteApi for WinRegistry {
    fn write_value(
        &self,
        hive: RegHive,
        path: &str,
        name: &str,
        value: &RegValue,
    ) -> Result<(), CoreError> {
        let key = open_existing(hive, path, KEY_SET_VALUE | KEY_QUERY_VALUE)?;
        let (ty, bytes) = value.to_raw();
        let name_w = HSTRING::from(name);
        // SAFETY: ключ відкритий з KEY_SET_VALUE, буфер `bytes` живий на час виклику.
        let status =
            unsafe { RegSetValueExW(key.0, &name_w, None, REG_VALUE_TYPE(ty), Some(&bytes)) };
        check(status, "RegSetValueExW")
    }

    fn delete_value(&self, hive: RegHive, path: &str, name: &str) -> Result<(), CoreError> {
        let key = match open_existing(hive, path, KEY_SET_VALUE) {
            Ok(k) => k,
            // Немає ключа — немає й значення.
            Err(CoreError::Os { code, .. }) if code == ERROR_FILE_NOT_FOUND.0 => return Ok(()),
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
}
