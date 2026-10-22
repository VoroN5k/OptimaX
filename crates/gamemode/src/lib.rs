//! Режим гри OptimaX.
//!
//! - [`profile`]: профілі ігор (що змінювати для кожної гри), перевірка на безпечність.
//! - [`session`]: застосування профілю під час гри й повне повернення після виходу.
//! - [`watcher`]: визначення запуску й закриття гри (чиста машина станів + опитування процесів).
//!
//! Усі зміни йдуть через рушій `optimax-tweaks`: журнал до зміни, перевірка, відкат.
//! Якщо OptimaX аварійно завершився під час гри, [`session::restore_leftovers`] при наступному
//! запуску поверне служби й пріоритети.

#![forbid(unsafe_code)]

pub mod profile;
pub mod session;
pub mod watcher;

pub use profile::{GamePriority, GameProfile, ProfileStore};
pub use session::{end_session, restore_leftovers, start_session, SessionReport};
pub use watcher::{scan, step, WatchEvent, WatchState};

/// Помилки режиму гри.
#[derive(Debug, thiserror::Error)]
pub enum GameModeError {
    /// Некоректний профіль.
    #[error("некоректний профіль: {0}")]
    InvalidProfile(String),
    /// Помилка сховища профілів.
    #[error("профілі: {0}")]
    Store(String),
    /// Помилка застосування змін.
    #[error(transparent)]
    Tweak(#[from] optimax_tweaks::TweakError),
}
