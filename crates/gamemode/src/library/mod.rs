//! Бібліотека ігор: встановлені ігри зі Steam, Epic, GOG і Battle.net.
//!
//! Лише читання конкретних файлів маніфестів і ключів реєстру — без обходу дисків і без мережі.
//! Парсери — чисті функції; доступ до файлів і реєстру — через трейти, тож усе тестується на моках.
//! Процес гри визначається не з маніфестів, а за текою встановлення ([`detect`]).

pub mod detect;
pub mod epic;
pub mod fs;
pub mod registry;
pub mod scan;
pub mod steam;
pub mod store;
pub mod vdf;

use serde::{Deserialize, Serialize};

/// Звідки гра.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Launcher {
    /// Steam.
    Steam,
    /// Epic Games Launcher.
    Epic,
    /// GOG.
    Gog,
    /// Battle.net.
    Battlenet,
    /// Додана вручну (лише ім'я exe, без теки).
    Manual,
}

/// Гра в бібліотеці.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GameEntry {
    /// Стабільний ідентифікатор: `steam:<appid>`, `epic:<AppName>`, `gog:<id>`, `bnet:<ключ>`, `manual:<exe>`.
    pub id: String,
    /// Назва.
    pub name: String,
    /// Лаунчер.
    pub launcher: Launcher,
    /// Тека встановлення (порожня для доданих вручну).
    pub install_dir: String,
    /// Ім'я exe, підтверджене визначенням процесу (або вказане вручну).
    pub exe: Option<String>,
    /// Ім'я exe з маніфесту — лише підказка, не підтверджене.
    pub exe_hint: Option<String>,
    /// Чи є локальна обкладинка.
    pub has_cover: bool,
    /// Людина приховала гру.
    pub hidden: bool,
}

/// Джерело, яке не вдалося прочитати (решта списку показується).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LibraryProblem {
    /// Лаунчер.
    pub launcher: Launcher,
    /// Що саме (файл, тека, ключ реєстру).
    pub what: String,
    /// Причина.
    pub reason: String,
}

/// Результат скану бібліотеки.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LibraryReport {
    /// Ігри, відсортовані за назвою.
    pub games: Vec<GameEntry>,
    /// Що не вдалося прочитати.
    pub problems: Vec<LibraryProblem>,
}

/// Помилки бібліотеки ігор.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LibraryError {
    /// Файл маніфесту не розібрано.
    #[error("не вдалося розібрати {what}: {reason}")]
    Parse {
        /// Що розбиралось (напр. «libraryfolders.vdf»).
        what: &'static str,
        /// Причина.
        reason: String,
    },
    /// Помилка читання чи запису файлу.
    #[error("помилка файлу: {0}")]
    Io(String),
}

impl LibraryError {
    pub(crate) fn parse(what: &'static str, reason: impl Into<String>) -> Self {
        Self::Parse {
            what,
            reason: reason.into(),
        }
    }
}

/// Тека для порівняння: `/` → `\`, без кінцевого роздільника, у нижньому регістрі (Unicode).
pub(crate) fn norm_dir(dir: &str) -> String {
    dir.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}
