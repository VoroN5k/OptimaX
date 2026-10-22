//! Читання реєстру за трейтом [`RegistryApi`], щоб логіку можна було тестувати моками.
//!
//! Запис з'явиться в крейті `tweaks` разом із журналом відкату.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::WinRegistry;

/// Корінь реєстру.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegHive {
    /// `HKEY_LOCAL_MACHINE`
    LocalMachine,
    /// `HKEY_CURRENT_USER`
    CurrentUser,
}

/// Значення реєстру з типом.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub trait RegistryApi {
    /// Читає значення. `Ok(None)`, якщо ключа чи значення немає.
    fn read_value(
        &self,
        hive: RegHive,
        path: &str,
        name: &str,
    ) -> Result<Option<RegValue>, CoreError>;
}

/// Реєстр у пам'яті для тестів і моків.
#[derive(Debug, Default, Clone)]
pub struct MemRegistry {
    values: HashMap<(RegHive, String, String), RegValue>,
}

impl MemRegistry {
    /// Порожній реєстр.
    pub fn new() -> Self {
        Self::default()
    }

    /// Встановлює значення (шлях і ім'я без урахування регістру, як у Windows).
    pub fn set(&mut self, hive: RegHive, path: &str, name: &str, value: RegValue) {
        self.values.insert(
            (hive, path.to_ascii_lowercase(), name.to_ascii_lowercase()),
            value,
        );
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
            .values
            .get(&(hive, path.to_ascii_lowercase(), name.to_ascii_lowercase()))
            .cloned())
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
