//! Модель запису історії.

use optimax_core::Text;
use optimax_diagnosis::{Bottleneck, CheckPlan};
use serde::{Deserialize, Serialize};

/// Режим монітора, на якому йшла гра.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplayState {
    /// Ширина, пікселів.
    pub width: u32,
    /// Висота, пікселів.
    pub height: u32,
    /// Частота, Гц.
    pub hz: u32,
}

/// Розмір і час зміни exe гри: оновлення гри змінює хоча б одне з них.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FileStamp {
    /// Розмір, байт.
    pub bytes: u64,
    /// Час останньої зміни (Unix-секунди).
    pub modified_unix: i64,
}

/// Знімок стану ПК на момент запису. `None` — не вдалося визначити (порівнюється як «невідомо»,
/// а не як зміна).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SystemState {
    /// Версія OptimaX, якою зроблено запис.
    pub optimax: String,
    /// Windows (назва й випуск).
    pub windows: Option<String>,
    /// Збірка ядра Windows.
    pub windows_build: Option<String>,
    /// Відеокарта.
    pub gpu: Option<String>,
    /// Версія драйвера відеокарти.
    pub driver: Option<String>,
    /// Режим головного монітора.
    pub display: Option<DisplayState>,
    /// План живлення (назва).
    pub power_plan: Option<String>,
    /// Версія BIOS/UEFI.
    pub bios: Option<String>,
    /// Робоча частота пам'яті, MT/s.
    pub ram_mts: Option<u32>,
    /// Апаратне планування GPU (HAGS): стан з реєстру.
    pub hags: Option<String>,
    /// Ігровий режим Windows.
    pub game_mode: Option<String>,
    /// Файл гри.
    pub game_file: Option<FileStamp>,
}

/// Що в стані ПК змінилося між двома записами.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StateField {
    /// Відеокарта.
    Gpu,
    /// Драйвер відеокарти.
    Driver,
    /// Windows.
    Windows,
    /// Збірка Windows.
    WindowsBuild,
    /// Режим монітора.
    Display,
    /// План живлення.
    PowerPlan,
    /// BIOS/UEFI.
    Bios,
    /// Частота пам'яті.
    Ram,
    /// HAGS.
    Hags,
    /// Ігровий режим Windows.
    GameMode,
    /// Файл гри (оновлення гри).
    GameFile,
    /// Версія OptimaX (могли змінитися правила аналізу).
    Optimax,
}

/// Одна зміна стану: поле, було → стало (як показувати, вирішує інтерфейс за `field`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StateChange {
    /// Що змінилося.
    pub field: StateField,
    /// Було.
    pub from: String,
    /// Стало.
    pub to: String,
}

/// Підсумок одного запису гри.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HistoryEntry {
    /// Ідентифікатор (ім'я файлу без розширення).
    pub id: String,
    /// Процес гри.
    pub game: String,
    /// Коли записано (Unix-мілісекунди).
    pub at_unix_ms: i64,
    /// Вузьке місце.
    pub bottleneck: Bottleneck,
    /// Заголовок головного вердикту.
    pub title: Text,
    /// Впевненість головного вердикту, %.
    pub confidence: u8,
    /// Середній FPS.
    pub avg_fps: Option<f64>,
    /// 1% low.
    pub low_1pct_fps: Option<f64>,
    /// 0.1% low.
    pub low_01pct_fps: Option<f64>,
    /// Кадрів у аналізі.
    pub frames: u64,
    /// Зараховано чистої гри, с.
    pub counted_s: f64,
    /// Розкид середнього FPS усередині запису, % (див. [`crate::fps_noise_pct`]).
    pub fps_noise_pct: Option<f64>,
    /// Стан ПК.
    pub state: SystemState,
    /// Повний запис збережено (можна відкрити радар і перерахувати).
    pub has_recording: bool,
    /// Мітка сцени, яку користувач вказав перед записом (`None` — не вказано).
    #[serde(default)]
    pub scene: Option<String>,
    /// Запис зроблено як перевірку дії («стало»): з яким записом «було» порівнювати й як.
    #[serde(default)]
    pub check: Option<CheckLink>,
}

/// Зв'язок запису «стало» із записом «було» в сценарії «Перевірити мою гру».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CheckLink {
    /// Id запису «було».
    pub baseline: String,
    /// За чим судити й варіанти A/B (з головної дії розповіді «було»).
    pub plan: CheckPlan,
}

/// Чи з однієї сцени два записи (за мітками користувача).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SceneMatch {
    /// Однакові мітки.
    Same,
    /// Різні мітки — FPS напряму не порівнянний.
    Different,
    /// В одного чи обох записів мітки немає.
    Unknown,
}

/// Чистить мітку сцени з інтерфейсу: без керівних символів, обрізано пробіли, до 40 символів.
pub fn clean_scene(raw: Option<&str>) -> Option<String> {
    let s: String = raw?.chars().filter(|c| !c.is_control()).take(40).collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Порівнює мітки сцен (без огляду на регістр і зайві пробіли).
pub fn scene_match(a: Option<&str>, b: Option<&str>) -> SceneMatch {
    match (a, b) {
        (Some(x), Some(y)) if x.trim().to_lowercase() == y.trim().to_lowercase() => {
            SceneMatch::Same
        }
        (Some(_), Some(_)) => SceneMatch::Different,
        _ => SceneMatch::Unknown,
    }
}
