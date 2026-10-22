//! Застосування змін системи з обов'язковим відкатом і перевірка ефекту A/B-тестом.
//!
//! Порядок дій при застосуванні ([`Engine::apply`]):
//! 1. перед першою зміною в сесії — точка відновлення Windows (не вдалося → питаємо користувача);
//! 2. повторне читання поточного значення й перевірка типу (ніколи не пишемо «наосліп»);
//! 3. запис у журнал **до** зміни (стан `Pending`), лише потім зміна;
//! 4. перевірка, що значення справді змінилось; збій → автоматичний відкат уже зроблених змін.
//!
//! Після аварійного завершення [`Engine::recover_pending`] доводить «завислі» записи до кінця.
//! `unsafe` дозволено лише в модулі `restore::windows` (FFI до System Restore).

pub mod ab;
pub mod catalog;
pub mod change;
pub mod engine;
pub mod journal;
pub mod restore;

pub use catalog::{Desired, TweakId, TweakInfo};
pub use change::{Plan, PlannedChange, SettingTarget, SettingValue};
pub use engine::{ApplyOutcome, Confirm, Engine};
pub use journal::{EntryState, FileStore, JournalEntry, JournalStore, MemStore};
pub use restore::{RestorePointApi, RestorePointStatus};

use optimax_core::CoreError;

/// Помилки застосування змін.
#[derive(Debug, thiserror::Error)]
pub enum TweakError {
    /// Помилка системного виклику або даних.
    #[error(transparent)]
    Core(#[from] CoreError),
    /// Поточне значення має інший тип, ніж очікується — запис заборонено.
    #[error("{location}: очікувався тип {expected}, у системі {found} — зміну не застосовано")]
    TypeMismatch {
        /// Де.
        location: String,
        /// Очікуваний тип.
        expected: &'static str,
        /// Фактичний тип.
        found: &'static str,
    },
    /// Значення змінилося між переглядом плану й застосуванням.
    #[error("{location}: значення змінилося після перегляду плану — переглянь зміни ще раз")]
    Stale {
        /// Де.
        location: String,
    },
    /// Після нашої зміни значення змінив хтось інший — відкат перезаписав би чужу зміну.
    #[error("{location}: значення змінено поза OptimaX після нашої зміни — відкат не виконано, щоб не затерти чужу зміну")]
    Conflict {
        /// Де.
        location: String,
    },
    /// Після запису система повернула інше значення.
    #[error("{location}: після запису значення не збігається з очікуваним")]
    VerifyFailed {
        /// Де.
        location: String,
    },
    /// Схеми живлення немає в системі.
    #[error("схеми живлення {0} немає в системі")]
    UnknownScheme(String),
    /// Дія заборонена правилами безпеки OptimaX.
    #[error("заборонено: {0}")]
    Forbidden(String),
    /// Для цієї зміни рушію не передано потрібний системний інтерфейс.
    #[error("недоступно: {0}")]
    Unavailable(&'static str),
    /// Користувач відмовився продовжувати.
    #[error("скасовано користувачем")]
    Cancelled,
    /// Немає запису журналу з таким номером або він у невідповідному стані.
    #[error("запис журналу #{0} не знайдено або його вже відкочено")]
    NoSuchEntry(u64),
    /// Помилка журналу.
    #[error("журнал змін: {0}")]
    Journal(String),
    /// Замало прогонів для A/B-висновку.
    #[error("для висновку потрібно щонайменше {min} прогонів «до» і «після» (є {a} і {b})")]
    NotEnoughRuns {
        /// Мінімум.
        min: usize,
        /// Прогонів «до».
        a: usize,
        /// Прогонів «після».
        b: usize,
    },
}
