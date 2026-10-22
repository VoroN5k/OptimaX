//! Спільний тип помилок.

use thiserror::Error;

/// Помилки, спільні для всіх крейтів OptimaX.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Виклик WinAPI повернув код помилки.
    #[error("{api} повернув помилку {code}")]
    Os {
        /// Назва функції WinAPI.
        api: &'static str,
        /// Код помилки Win32.
        code: u32,
    },
    /// Дані мають неочікуваний формат (тип значення реєстру, таблиця прошивки тощо).
    #[error("некоректні дані: {0}")]
    InvalidData(String),
    /// Функція недоступна на цій платформі.
    #[error("не підтримується на цій платформі: {0}")]
    Unsupported(&'static str),
    /// Об'єкта (процесу, служби) більше немає.
    #[error("не знайдено: {0}")]
    NotFound(String),
    /// Помилка розбору JSON.
    #[error("помилка JSON: {0}")]
    Json(#[from] serde_json::Error),
}
