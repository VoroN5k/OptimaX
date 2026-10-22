//! Помилки команд у форматі, зручному для UI.

use optimax_tweaks::TweakError;
use serde::Serialize;

/// Помилка команди.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum CmdError {
    /// Точку відновлення створити не вдалося — потрібне явне рішення користувача.
    RestorePoint(String),
    /// Дію скасовано.
    Cancelled(String),
    /// Заборонено правилами безпеки.
    Forbidden(String),
    /// Об'єкт (план, сесію) не знайдено.
    NotFound(String),
    /// Гра не малювала кадрів під час запису (не запущена, згорнута, на паузі).
    NoFrames(String),
    /// Інша помилка.
    Failed(String),
}

/// Результат команди.
pub type CmdResult<T> = Result<T, CmdError>;

impl From<TweakError> for CmdError {
    fn from(e: TweakError) -> Self {
        match e {
            TweakError::Cancelled => Self::Cancelled(e.to_string()),
            TweakError::Forbidden(_) | TweakError::TypeMismatch { .. } => {
                Self::Forbidden(e.to_string())
            }
            TweakError::NoSuchEntry(_) => Self::NotFound(e.to_string()),
            other => Self::Failed(other.to_string()),
        }
    }
}

macro_rules! failed_from {
    ($($t:ty),*) => {$(
        impl From<$t> for CmdError {
            fn from(e: $t) -> Self {
                Self::Failed(e.to_string())
            }
        }
    )*};
}

impl From<optimax_frametime::FrametimeError> for CmdError {
    fn from(e: optimax_frametime::FrametimeError) -> Self {
        match e {
            optimax_frametime::FrametimeError::NoFrames(details) => Self::NoFrames(details),
            other => Self::Failed(other.to_string()),
        }
    }
}

failed_from!(
    optimax_core::CoreError,
    optimax_gamemode::GameModeError,
    optimax_metrics::MetricsError,
    anyhow::Error,
    std::io::Error,
    serde_json::Error,
    tauri::Error
);
