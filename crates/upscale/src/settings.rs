//! Налаштування апскейлу.

use crate::geometry::FitMode;
use serde::{Deserialize, Serialize};

/// Фільтр масштабування.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    /// Найближчий піксель: чіткі «квадратні» пікселі (для піксель-арту та цілих множників).
    Nearest,
    /// Білінійна інтерполяція: гладко, але трохи розмито.
    #[default]
    Bilinear,
}

/// Налаштування сеансу апскейлу.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct UpscaleSettings {
    /// Фільтр.
    pub filter: Filter,
    /// Як вписати кадр в екран.
    pub fit: FitMode,
    /// Захоплювати курсор разом із кадром гри.
    pub capture_cursor: bool,
}

impl Default for UpscaleSettings {
    fn default() -> Self {
        Self {
            filter: Filter::Bilinear,
            fit: FitMode::Aspect,
            capture_cursor: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() -> Result<(), serde_json::Error> {
        let s: UpscaleSettings = serde_json::from_str(r#"{"filter":"nearest"}"#)?;
        assert_eq!(s.filter, Filter::Nearest);
        assert_eq!(s.fit, FitMode::Aspect);
        assert!(s.capture_cursor);
        Ok(())
    }
}
