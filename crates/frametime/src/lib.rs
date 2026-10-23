//! Frametime для OptimaX: вбудований PresentMon, розбір CSV, статистика FPS і пошук піків.
//!
//! - [`csv`]: розбір CSV PresentMon 1.x/2.x за назвами колонок (порядок і набір колонок
//!   відрізняються між версіями, тому позиції не використовуються).
//! - [`stats`]: avg FPS, 1% / 0.1% low, перцентилі, піки frametime.
//! - [`presentmon`]: вбудований бінарник із перевіркою SHA-256 перед **кожним** запуском.
//! - [`recording`]: одночасний запис кадрів і системних метрик на спільній шкалі QPC.
//!
//! `unsafe` дозволено лише в модулі `acl` (FFI до WinAPI для прав доступу до каталогу).

pub mod csv;
pub mod frame;
pub mod gate;
pub mod presentmon;
pub mod recording;
pub mod stats;

#[cfg(windows)]
mod acl;

pub use frame::{FrameRecord, FrameTimestamp};
pub use gate::{EndReason, Gate, Observation, Phase, StartMode};
pub use presentmon::{BundledPresentMon, Target};
pub use recording::{
    record, record_with, CancelToken, RecordNotices, RecordOptions, RecordProgress, Recording,
};
pub use stats::{find_spikes, frame_stats, main_swap_chain, FrameStats, Spike};

use optimax_core::CoreError;

/// Помилки модуля frametime.
#[derive(Debug, thiserror::Error)]
pub enum FrametimeError {
    /// У заголовку CSV немає обов'язкової колонки.
    #[error("у CSV PresentMon немає колонки {0}")]
    MissingColumn(&'static str),
    /// Значення не вдалося розібрати.
    #[error("некоректне значення в колонці {column}: «{value}»")]
    BadValue {
        /// Колонка.
        column: &'static str,
        /// Сире значення.
        value: String,
    },
    /// Рядок має менше полів, ніж заголовок.
    #[error("рядок CSV має {got} полів замість {expected}")]
    ShortLine {
        /// Скільки полів у рядку.
        got: usize,
        /// Скільки очікувалось.
        expected: usize,
    },
    /// Вбудований або встановлений PresentMon не пройшов перевірку SHA-256.
    #[error("SHA-256 PresentMon не збігається (очікувано {expected}, отримано {actual}) — запуск заборонено")]
    HashMismatch {
        /// Очікуваний хеш.
        expected: String,
        /// Фактичний хеш.
        actual: String,
    },
    /// Некоректна ціль запису.
    #[error("некоректна ціль запису: {0}")]
    InvalidTarget(String),
    /// Глобальну гарячу клавішу не вдалося зареєструвати (зайнята іншою програмою).
    #[error("гаряча клавіша Ctrl+Shift+F9 недоступна: {0}")]
    Hotkey(String),
    /// Зараховування так і не почалось (гру не відкрили / клавішу не натиснули / скасовано).
    #[error("запис не почався: {0:?}")]
    NotStarted(gate::EndReason),
    /// PresentMon не записав жодного кадру.
    #[error("PresentMon не записав жодного кадру. Повідомлення PresentMon: {0}")]
    NoFrames(String),
    /// Помилка вводу-виводу.
    #[error("помилка вводу-виводу: {0}")]
    Io(#[from] std::io::Error),
    /// Помилка спільного шару.
    #[error(transparent)]
    Core(#[from] CoreError),
    /// Помилка семплера метрик.
    #[error(transparent)]
    Metrics(#[from] optimax_metrics::MetricsError),
}
