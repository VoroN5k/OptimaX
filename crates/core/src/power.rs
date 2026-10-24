//! Схеми живлення за трейтом [`PowerApi`] (Windows — [`WinPower`], тести — [`MemPower`]).

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, PoisonError};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::WinPower;

/// GUID стандартних схем Windows.
pub mod guids {
    /// «Економія енергії»
    pub const POWER_SAVER: &str = "a1841308-3541-4fab-bc81-f71556f20b4a";
    /// «Збалансована»
    pub const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
    /// «Висока продуктивність»
    pub const HIGH_PERFORMANCE: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
    /// «Максимальна продуктивність»
    pub const ULTIMATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";
}

/// GUID оверлейних режимів живлення (недокументований API powrprof.dll).
/// Нульовий GUID = збалансований (без активного оверлею).
pub mod overlay_guids {
    /// «Економія» (Battery saver).
    pub const ECONOMY: &str = "961cc777-2547-4f9d-8174-7d86181b8a7a";
    /// «Краща продуктивність».
    pub const BETTER_PERFORMANCE: &str = "3af9b8d9-7c97-431d-ad78-34a8bfea439f";
    /// «Найкраща продуктивність» (Ultimate Performance overlay).
    pub const BEST_PERFORMANCE: &str = "ded574b5-45a0-4f42-8737-46345c09c238";
}

/// Схема живлення.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PowerScheme {
    /// GUID у нижньому регістрі.
    pub guid: String,
    /// Назва (локалізована Windows).
    pub name: Option<String>,
}

/// Форматує GUID із `u128` у канонічний вигляд у нижньому регістрі.
pub fn format_guid(v: u128) -> String {
    let h = format!("{v:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Розбирає GUID `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` (регістр не важливий, фігурні дужки дозволені).
pub fn parse_guid(s: &str) -> Option<u128> {
    let t = s.trim().trim_start_matches('{').trim_end_matches('}');
    let groups: Vec<&str> = t.split('-').collect();
    let lens = [8, 4, 4, 4, 12];
    if groups.len() != 5 || groups.iter().zip(lens).any(|(g, l)| g.len() != l) {
        return None;
    }
    u128::from_str_radix(&groups.concat(), 16).ok()
}

/// Керування схемами живлення.
pub trait PowerApi: Send + Sync {
    /// GUID активної схеми.
    fn active_scheme(&self) -> Result<String, CoreError>;
    /// Робить схему активною. Схема має існувати.
    fn set_active_scheme(&self, guid: &str) -> Result<(), CoreError>;
    /// Усі схеми, наявні в системі.
    fn schemes(&self) -> Result<Vec<PowerScheme>, CoreError>;
    /// Назва схеми.
    fn scheme_name(&self, guid: &str) -> Option<String> {
        self.schemes()
            .ok()?
            .into_iter()
            .find(|s| s.guid.eq_ignore_ascii_case(guid))
            .and_then(|s| s.name)
    }

    /// Чи доступний API оверлейного режиму живлення (недокументований powrprof.dll).
    fn overlay_supported(&self) -> bool {
        false
    }

    /// GUID поточного оверлейного режиму; `None` = збалансований / не встановлено.
    fn overlay_scheme(&self) -> Result<Option<String>, CoreError> {
        Ok(None)
    }

    /// Встановлює оверлейний режим. `None` = скинути до збалансованого.
    fn set_overlay_scheme(&self, guid: Option<&str>) -> Result<(), CoreError> {
        let _ = guid;
        Err(CoreError::InvalidData(
            "overlay scheme не підтримується на цій конфігурації".into(),
        ))
    }
}

/// Схеми живлення в пам'яті для тестів.
#[derive(Debug)]
pub struct MemPower {
    active: Mutex<String>,
    schemes: Vec<PowerScheme>,
}

impl MemPower {
    /// Стандартні «Збалансована» і «Висока продуктивність», активна — `active`.
    pub fn new(active: &str) -> Self {
        Self {
            active: Mutex::new(active.to_string()),
            schemes: vec![
                PowerScheme {
                    guid: guids::BALANCED.into(),
                    name: Some("Balanced".into()),
                },
                PowerScheme {
                    guid: guids::HIGH_PERFORMANCE.into(),
                    name: Some("High performance".into()),
                },
            ],
        }
    }
}

impl PowerApi for MemPower {
    fn active_scheme(&self) -> Result<String, CoreError> {
        Ok(self
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    fn set_active_scheme(&self, guid: &str) -> Result<(), CoreError> {
        if !self
            .schemes
            .iter()
            .any(|s| s.guid.eq_ignore_ascii_case(guid))
        {
            return Err(CoreError::InvalidData(format!("немає схеми {guid}")));
        }
        *self.active.lock().unwrap_or_else(PoisonError::into_inner) = guid.to_ascii_lowercase();
        Ok(())
    }

    fn schemes(&self) -> Result<Vec<PowerScheme>, CoreError> {
        Ok(self.schemes.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_roundtrip() {
        let v = 0x381b4222_f694_41f0_9685_ff5bb260df2e;
        assert_eq!(format_guid(v), guids::BALANCED);
        assert_eq!(parse_guid(guids::BALANCED), Some(v));
        assert_eq!(
            parse_guid("{381B4222-F694-41F0-9685-FF5BB260DF2E}"),
            Some(v)
        );
        assert_eq!(parse_guid("381b4222-f694-41f0-9685"), None);
        assert_eq!(parse_guid("zzzzzzzz-f694-41f0-9685-ff5bb260df2e"), None);
    }

    #[test]
    fn mem_power_switches_only_to_existing() {
        let p = MemPower::new(guids::HIGH_PERFORMANCE);
        p.set_active_scheme(guids::BALANCED).unwrap();
        assert_eq!(p.active_scheme().unwrap(), guids::BALANCED);
        assert!(p.set_active_scheme(guids::ULTIMATE).is_err());
        assert_eq!(p.scheme_name(guids::BALANCED).as_deref(), Some("Balanced"));
    }
}
