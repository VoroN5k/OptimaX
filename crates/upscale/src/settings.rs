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
    Bilinear,
    /// Lanczos-3: чіткіше за білінійну, без ореолів біля країв.
    Lanczos,
    /// AMD FSR 1 (EASU + RCAS): збільшення з урахуванням країв і адаптивна різкість.
    #[default]
    Fsr,
    /// NVIDIA Image Scaling: напрямлене збільшення з різкістю. Працює до ×2,
    /// більший масштаб дотягує білінійний прохід.
    Nis,
}

/// Різкість за замовчуванням (0…1).
pub const DEFAULT_SHARPNESS: f32 = 0.5;

/// Налаштування сеансу апскейлу.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct UpscaleSettings {
    /// Фільтр.
    pub filter: Filter,
    /// Як вписати кадр в екран.
    pub fit: FitMode,
    /// Різкість 0…1 для FSR і NIS (0 — без додаткової різкості, 1 — максимальна).
    pub sharpness: f32,
    /// Захоплювати курсор разом із кадром гри.
    pub capture_cursor: bool,
}

impl Default for UpscaleSettings {
    fn default() -> Self {
        Self {
            filter: Filter::Fsr,
            fit: FitMode::Aspect,
            sharpness: DEFAULT_SHARPNESS,
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
        assert_eq!(s.sharpness, DEFAULT_SHARPNESS);
        assert!(s.capture_cursor);
        Ok(())
    }

    #[test]
    fn filters_use_snake_case() -> Result<(), serde_json::Error> {
        assert_eq!(serde_json::to_string(&Filter::Fsr)?, r#""fsr""#);
        assert_eq!(serde_json::to_string(&Filter::Lanczos)?, r#""lanczos""#);
        assert_eq!(serde_json::from_str::<Filter>(r#""nis""#)?, Filter::Nis);
        Ok(())
    }
}
