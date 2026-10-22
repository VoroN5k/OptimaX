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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
