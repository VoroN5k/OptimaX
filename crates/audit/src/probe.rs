//! Трейт проби системи та читання налаштувань Windows із реєстру.

use crate::model::{
    BatteryInfo, DisplayInfo, FirmwareInfo, GameModeState, GpuAdapter, HagsInfo, HagsState,
    PowerPlan, SystemEvent,
};
use optimax_core::registry::{RegHive, RegValue, RegistryApi};
use optimax_core::CoreError;

/// Ключ ігрового режиму.
pub const GAME_MODE_KEY: &str = r"Software\Microsoft\GameBar";
/// Значення ігрового режиму (DWORD, 1/0).
pub const GAME_MODE_VALUE: &str = "AutoGameModeEnabled";
/// Ключ налаштувань графічного ядра.
pub const HAGS_KEY: &str = r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers";
/// Значення HAGS (DWORD, 2 — увімкнено, 1 — вимкнено).
pub const HAGS_VALUE: &str = "HwSchMode";

/// Джерело даних для аудиту. Реалізація для Windows — [`crate::WindowsProbe`]; у тестах — моки.
pub trait SystemProbe {
    /// Таблиця SMBIOS: BIOS і модулі пам'яті.
    fn firmware(&self) -> Result<FirmwareInfo, CoreError>;
    /// Активні монітори.
    fn displays(&self) -> Result<Vec<DisplayInfo>, CoreError>;
    /// Фізичні відеоадаптери.
    fn gpus(&self) -> Result<Vec<GpuAdapter>, CoreError>;
    /// Активна схема живлення.
    fn power_plan(&self) -> Result<PowerPlan, CoreError>;
    /// Ігровий режим.
    fn game_mode(&self) -> Result<GameModeState, CoreError>;
    /// HAGS: реєстр + реальний стан адаптерів.
    fn hags(&self) -> Result<HagsInfo, CoreError>;
    /// Стан батареї. `Ok(None)` — батареї немає (десктоп або немає підтримки).
    fn battery_info(&self) -> Result<Option<BatteryInfo>, CoreError> {
        Ok(None)
    }
    /// Події TDR і WHEA з журналу System за останні `days` днів.
    /// `Ok(None)` — джерело недоступне (мок без даних); перевірка тоді не виконується.
    fn hardware_events(&self, _days: u32) -> Result<Option<Vec<SystemEvent>>, CoreError> {
        Ok(None)
    }
}

fn expect_dword(value: Option<RegValue>, name: &str) -> Result<Option<u32>, CoreError> {
    match value {
        None => Ok(None),
        Some(RegValue::Dword(v)) => Ok(Some(v)),
        Some(other) => Err(CoreError::InvalidData(format!(
            "{name}: очікувався REG_DWORD, отримано {other:?}"
        ))),
    }
}

/// Читає стан ігрового режиму з реєстру поточного користувача.
pub fn read_game_mode(reg: &dyn RegistryApi) -> Result<GameModeState, CoreError> {
    let v = reg.read_value(RegHive::CurrentUser, GAME_MODE_KEY, GAME_MODE_VALUE)?;
    Ok(match expect_dword(v, GAME_MODE_VALUE)? {
        None => GameModeState::Default,
        Some(0) => GameModeState::Disabled,
        Some(_) => GameModeState::Enabled,
    })
}

/// Читає стан HAGS.
pub fn read_hags(reg: &dyn RegistryApi) -> Result<HagsState, CoreError> {
    let v = reg.read_value(RegHive::LocalMachine, HAGS_KEY, HAGS_VALUE)?;
    Ok(match expect_dword(v, HAGS_VALUE)? {
        None => HagsState::NotSet,
        Some(2) => HagsState::Enabled,
        Some(1) => HagsState::Disabled,
        Some(other) => HagsState::Unexpected(other),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::registry::MemRegistry;

    #[test]
    fn game_mode_from_registry() {
        let mut r = MemRegistry::new();
        assert_eq!(read_game_mode(&r).unwrap(), GameModeState::Default);
        r.set(
            RegHive::CurrentUser,
            GAME_MODE_KEY,
            GAME_MODE_VALUE,
            RegValue::Dword(0),
        );
        assert_eq!(read_game_mode(&r).unwrap(), GameModeState::Disabled);
        r.set(
            RegHive::CurrentUser,
            GAME_MODE_KEY,
            GAME_MODE_VALUE,
            RegValue::Dword(1),
        );
        assert_eq!(read_game_mode(&r).unwrap(), GameModeState::Enabled);
        r.set(
            RegHive::CurrentUser,
            GAME_MODE_KEY,
            GAME_MODE_VALUE,
            RegValue::String("1".into()),
        );
        assert!(read_game_mode(&r).is_err());
    }

    #[test]
    fn hags_from_registry() {
        let mut r = MemRegistry::new();
        assert_eq!(read_hags(&r).unwrap(), HagsState::NotSet);
        r.set(
            RegHive::LocalMachine,
            HAGS_KEY,
            HAGS_VALUE,
            RegValue::Dword(2),
        );
        assert_eq!(read_hags(&r).unwrap(), HagsState::Enabled);
        r.set(
            RegHive::LocalMachine,
            HAGS_KEY,
            HAGS_VALUE,
            RegValue::Dword(1),
        );
        assert_eq!(read_hags(&r).unwrap(), HagsState::Disabled);
        r.set(
            RegHive::LocalMachine,
            HAGS_KEY,
            HAGS_VALUE,
            RegValue::Dword(9),
        );
        assert_eq!(read_hags(&r).unwrap(), HagsState::Unexpected(9));
    }
}
