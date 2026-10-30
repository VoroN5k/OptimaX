//! Налаштування інтерфейсу (`%LOCALAPPDATA%\OptimaX\settings.json`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Якість фонової анімації.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AnimationLevel {
    /// Повна.
    #[default]
    High,
    /// Менше точок і кадрів.
    Low,
    /// Вимкнена.
    Off,
}

/// Налаштування.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Settings {
    /// Фонова анімація.
    #[serde(default)]
    pub animation: AnimationLevel,
    /// Економний режим: без анімацій, рідше оновлення метрик.
    #[serde(default)]
    pub economy: bool,
    /// Як починати запис гри (діагностика й A/B).
    #[serde(default)]
    pub record_start: optimax_frametime::StartMode,
    /// Звукові сигнали на старті й зупинці запису.
    #[serde(default = "yes")]
    pub record_sound: bool,
    /// Загальні налаштування апскейлу (профіль гри може мати власні).
    #[serde(default)]
    pub upscale: optimax_upscale::UpscaleSettings,
    /// Гаряча клавіша Ctrl+Shift+F10 для апскейлу (працює, поки відкрита OptimaX).
    #[serde(default = "yes")]
    pub upscale_hotkey: bool,
    /// Очищення диска.
    #[serde(default)]
    pub cleanup: CleanupSettings,
    /// Перевіряти оновлення при запуску (мережевий запит до GitHub; за замовчуванням вимкнено).
    #[serde(default)]
    pub update_check_on_start: bool,
}

/// Налаштування очищення.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CleanupSettings {
    /// Скільки днів файли 🟡 лежать у карантині.
    #[serde(default = "quarantine_days")]
    pub quarantine_days: u32,
    /// Диск для карантину (`D:\`); `None` — системний (`%LOCALAPPDATA%\OptimaX\Quarantine`).
    #[serde(default)]
    pub quarantine_drive: Option<String>,
    /// Автоматично видаляти карантин після терміну (при старті програми, із записом у журналі).
    #[serde(default)]
    pub auto_purge: bool,
}

fn quarantine_days() -> u32 {
    7
}

impl Default for CleanupSettings {
    fn default() -> Self {
        Self {
            quarantine_days: quarantine_days(),
            quarantine_drive: None,
            auto_purge: false,
        }
    }
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            animation: AnimationLevel::default(),
            economy: false,
            record_start: optimax_frametime::StartMode::Auto,
            record_sound: true,
            upscale: optimax_upscale::UpscaleSettings::default(),
            upscale_hotkey: true,
            cleanup: CleanupSettings::default(),
            update_check_on_start: false,
        }
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("OptimaX").join("settings.json"))
}

impl Settings {
    /// Завантажує або повертає типові.
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Зберігає.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = path() else {
            return Ok(());
        };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(p, json)
    }
}
