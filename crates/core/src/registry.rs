//! Доступ до реєстру за трейтами [`RegistryApi`] (читання) і [`RegistryWriteApi`] (запис),
//! щоб логіку можна було тестувати моками.
//!
//! Запис дозволено лише в **уже наявні** ключі: нові ключі не створюються, щоб відкат завжди
//! зводився до «повернути значення» або «видалити значення». Використовує його лише крейт
//! `tweaks`, який перед кожним записом зберігає старе значення в журнал.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::WinRegistry;

/// Корінь реєстру.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum RegHive {
    /// `HKEY_LOCAL_MACHINE`
    LocalMachine,
    /// `HKEY_CURRENT_USER`
    CurrentUser,
}

/// Значення реєстру з типом.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", content = "value")]
pub enum RegValue {
    /// `REG_DWORD`
    Dword(u32),
    /// `REG_QWORD`
    Qword(u64),
    /// `REG_SZ`
    String(String),
    /// `REG_EXPAND_SZ` (без розгортання змінних).
    ExpandString(String),
    /// `REG_MULTI_SZ`
    MultiString(Vec<String>),
    /// `REG_BINARY` та інші типи.
    Binary(Vec<u8>),
}

const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_BINARY: u32 = 3;
const REG_DWORD: u32 = 4;
const REG_MULTI_SZ: u32 = 7;
const REG_QWORD: u32 = 11;

impl RegValue {
    /// Розбирає сирі байти значення за кодом типу `REG_*`.
    pub fn from_raw(reg_type: u32, data: &[u8]) -> Result<Self, CoreError> {
        match reg_type {
            REG_DWORD => {
                let bytes: [u8; 4] =
                    data.get(..4)
                        .and_then(|b| b.try_into().ok())
                        .ok_or_else(|| {
                            CoreError::InvalidData("REG_DWORD коротший за 4 байти".into())
                        })?;
                Ok(Self::Dword(u32::from_le_bytes(bytes)))
            }
            REG_QWORD => {
                let bytes: [u8; 8] = data
                    .get(..8)
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| CoreError::InvalidData("REG_QWORD коротший за 8 байт".into()))?;
                Ok(Self::Qword(u64::from_le_bytes(bytes)))
            }
            REG_SZ => Ok(Self::String(utf16_until_nul(data))),
            REG_EXPAND_SZ => Ok(Self::ExpandString(utf16_until_nul(data))),
            REG_MULTI_SZ => {
                let units = to_u16(data);
                let items = units
                    .split(|&u| u == 0)
                    .filter(|s| !s.is_empty())
                    .map(String::from_utf16_lossy)
                    .collect();
                Ok(Self::MultiString(items))
            }
            REG_BINARY => Ok(Self::Binary(data.to_vec())),
            _ => Ok(Self::Binary(data.to_vec())),
        }
    }

    /// DWORD-значення, якщо тип збігається.
    pub fn as_dword(&self) -> Option<u32> {
        match self {
            Self::Dword(v) => Some(*v),
            _ => None,
        }
    }

    /// Код типу `REG_*` і сирі байти для запису (зворотне до [`RegValue::from_raw`]).
    pub fn to_raw(&self) -> (u32, Vec<u8>) {
        fn wide(s: &str) -> Vec<u8> {
            s.encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect()
        }
        match self {
            Self::Dword(v) => (REG_DWORD, v.to_le_bytes().to_vec()),
            Self::Qword(v) => (REG_QWORD, v.to_le_bytes().to_vec()),
            Self::String(s) => (REG_SZ, wide(s)),
            Self::ExpandString(s) => (REG_EXPAND_SZ, wide(s)),
            Self::MultiString(items) => {
                let mut out: Vec<u8> = items.iter().flat_map(|s| wide(s)).collect();
                out.extend_from_slice(&[0, 0]);
                (REG_MULTI_SZ, out)
            }
            Self::Binary(b) => (REG_BINARY, b.clone()),
        }
    }

    /// Назва типу для показу користувачу (`REG_DWORD` тощо).
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Dword(_) => "REG_DWORD",
            Self::Qword(_) => "REG_QWORD",
            Self::String(_) => "REG_SZ",
            Self::ExpandString(_) => "REG_EXPAND_SZ",
            Self::MultiString(_) => "REG_MULTI_SZ",
            Self::Binary(_) => "REG_BINARY",
        }
    }

    /// Рядок для `REG_SZ`/`REG_EXPAND_SZ`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) | Self::ExpandString(s) => Some(s),
            _ => None,
        }
    }
}

fn to_u16(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn utf16_until_nul(data: &[u8]) -> String {
    let units = to_u16(data);
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// Доступ до реєстру лише для читання.
/// `Send + Sync`: один екземпляр використовують UI і фонові потоки.
pub trait RegistryApi: Send + Sync {
    /// Читає значення. `Ok(None)`, якщо ключа чи значення немає.
    fn read_value(
        &self,
        hive: RegHive,
        path: &str,
        name: &str,
    ) -> Result<Option<RegValue>, CoreError>;

    /// Імена всіх значень ключа (порожньо, якщо ключа немає). Порядок — як віддає система.
    fn value_names(&self, hive: RegHive, path: &str) -> Result<Vec<String>, CoreError>;
}

/// Запис у реєстр. Ключ має вже існувати — інакше помилка, ключі не створюються.
pub trait RegistryWriteApi: RegistryApi {
    /// Записує значення в наявний ключ.
    fn write_value(
        &self,
        hive: RegHive,
        path: &str,
        name: &str,
        value: &RegValue,
    ) -> Result<(), CoreError>;

    /// Видаляє значення. Якщо його вже немає — не помилка.
    fn delete_value(&self, hive: RegHive, path: &str, name: &str) -> Result<(), CoreError>;
}

type ValueKey = (RegHive, String, String);

#[derive(Debug, Default)]
struct MemState {
    keys: HashSet<(RegHive, String)>,
    values: HashMap<ValueKey, RegValue>,
}

/// Реєстр у пам'яті для тестів і моків (з тими самими правилами, що й справжній).
#[derive(Debug, Default)]
pub struct MemRegistry {
    state: Mutex<MemState>,
}

fn value_key(hive: RegHive, path: &str, name: &str) -> ValueKey {
    (hive, path.to_ascii_lowercase(), name.to_ascii_lowercase())
}

impl MemRegistry {
    /// Порожній реєстр.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, MemState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Створює порожній ключ.
    pub fn create_key(&mut self, hive: RegHive, path: &str) {
        self.lock().keys.insert((hive, path.to_ascii_lowercase()));
    }

    /// Встановлює значення, створюючи ключ (шлях і ім'я без урахування регістру, як у Windows).
    pub fn set(&mut self, hive: RegHive, path: &str, name: &str, value: RegValue) {
        let mut st = self.lock();
        st.keys.insert((hive, path.to_ascii_lowercase()));
        st.values.insert(value_key(hive, path, name), value);
    }
}

impl RegistryApi for MemRegistry {
    fn read_value(
        &self,
        hive: RegHive,
        path: &str,
        name: &str,
    ) -> Result<Option<RegValue>, CoreError> {
        Ok(self
            .lock()
            .values
            .get(&value_key(hive, path, name))
            .cloned())
    }

    fn value_names(&self, hive: RegHive, path: &str) -> Result<Vec<String>, CoreError> {
        let path = path.to_ascii_lowercase();
        let mut names: Vec<String> = self
            .lock()
            .values
            .keys()
            .filter(|(h, p, _)| *h == hive && *p == path)
            .map(|(_, _, n)| n.clone())
            .collect();
        names.sort();
        Ok(names)
    }
}

impl RegistryWriteApi for MemRegistry {
    fn write_value(
        &self,
        hive: RegHive,
        path: &str,
        name: &str,
        value: &RegValue,
    ) -> Result<(), CoreError> {
        let mut st = self.lock();
        if !st.keys.contains(&(hive, path.to_ascii_lowercase())) {
            return Err(CoreError::Os {
                api: "RegOpenKeyExW",
                code: 2,
            });
        }
        st.values.insert(value_key(hive, path, name), value.clone());
        Ok(())
    }

    fn delete_value(&self, hive: RegHive, path: &str, name: &str) -> Result<(), CoreError> {
        self.lock().values.remove(&value_key(hive, path, name));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16z(parts: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in parts {
            for u in p.encode_utf16().chain(std::iter::once(0)) {
                out.extend_from_slice(&u.to_le_bytes());
            }
        }
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn parses_dword_and_qword() {
        assert_eq!(
            RegValue::from_raw(4, &[2, 0, 0, 0]).unwrap(),
            RegValue::Dword(2)
        );
        assert_eq!(
            RegValue::from_raw(11, &1u64.to_le_bytes()).unwrap(),
            RegValue::Qword(1)
        );
        assert!(RegValue::from_raw(4, &[1, 2]).is_err());
    }

    #[test]
    fn parses_strings() {
        let sz = utf16z(&["Привіт"]);
        assert_eq!(
            RegValue::from_raw(1, &sz).unwrap(),
            RegValue::String("Привіт".into())
        );
        let multi = utf16z(&["a", "bc"]);
        assert_eq!(
            RegValue::from_raw(7, &multi).unwrap(),
            RegValue::MultiString(vec!["a".into(), "bc".into()])
        );
    }

    #[test]
    fn raw_roundtrip() {
        for v in [
            RegValue::Dword(0xDEAD_BEEF),
            RegValue::Qword(u64::MAX),
            RegValue::String("Привіт".into()),
            RegValue::ExpandString("%SystemRoot%".into()),
            RegValue::MultiString(vec!["a".into(), "bc".into()]),
            RegValue::Binary(vec![1, 2, 3]),
        ] {
            let (ty, bytes) = v.to_raw();
            assert_eq!(RegValue::from_raw(ty, &bytes).unwrap(), v);
        }
        assert_eq!(RegValue::Dword(1).type_name(), "REG_DWORD");
    }

    #[test]
    fn mem_registry_writes_only_into_existing_keys() {
        let mut r = MemRegistry::new();
        r.create_key(RegHive::CurrentUser, r"Software\Exists");
        r.write_value(
            RegHive::CurrentUser,
            r"software\exists",
            "V",
            &RegValue::Dword(3),
        )
        .unwrap();
        assert_eq!(
            r.read_value(RegHive::CurrentUser, r"Software\Exists", "v")
                .unwrap(),
            Some(RegValue::Dword(3))
        );
        assert!(r
            .write_value(
                RegHive::CurrentUser,
                r"Software\Missing",
                "V",
                &RegValue::Dword(1)
            )
            .is_err());
        r.delete_value(RegHive::CurrentUser, r"Software\Exists", "V")
            .unwrap();
        r.delete_value(RegHive::CurrentUser, r"Software\Exists", "V")
            .unwrap();
        assert_eq!(
            r.read_value(RegHive::CurrentUser, r"Software\Exists", "V")
                .unwrap(),
            None
        );
    }

    #[test]
    fn mem_registry_is_case_insensitive() {
        let mut r = MemRegistry::new();
        r.set(
            RegHive::CurrentUser,
            r"Software\X",
            "Val",
            RegValue::Dword(1),
        );
        assert_eq!(
            r.read_value(RegHive::CurrentUser, r"SOFTWARE\x", "val")
                .unwrap(),
            Some(RegValue::Dword(1))
        );
        assert_eq!(
            r.read_value(RegHive::LocalMachine, r"Software\X", "Val")
                .unwrap(),
            None
        );
    }
}
