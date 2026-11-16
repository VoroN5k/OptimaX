//! Типи обміну з інтерфейсом, специфічні для застосунку.

use optimax_audit::model::{DisplayInfo, FirmwareInfo, GpuAdapter};
use optimax_core::Text;
use optimax_diagnosis::Diagnosis;
use optimax_tweaks::{Plan, RestorePointStatus};
use serde::{Deserialize, Serialize};

/// Відомості про програму.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AppInfo {
    /// Версія.
    pub version: String,
    /// Запущено з правами адміністратора.
    pub elevated: bool,
    /// Служба OptimaX доступна цій копії програми (зміни системи — через неї).
    pub service: bool,
    /// Версія вбудованого PresentMon.
    pub presentmon_version: String,
}

/// Невдалий відкат.
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RollbackFailure {
    /// Запис журналу.
    pub id: u64,
    /// Причина.
    pub error: String,
}

/// Результат відкату.
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RollbackResult {
    /// Відкочені записи.
    pub rolled_back: Vec<u64>,
    /// Невдалі.
    pub failed: Vec<RollbackFailure>,
}

impl From<optimax_tweaks::engine::RollbackSummary> for RollbackResult {
    fn from(s: optimax_tweaks::engine::RollbackSummary) -> Self {
        Self {
            rolled_back: s.rolled_back,
            failed: s
                .failed
                .into_iter()
                .map(|(id, error)| RollbackFailure { id, error })
                .collect(),
        }
    }
}

/// Що зроблено при старті програми (відновлення після збою).
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct StartupReport {
    /// Оброблено «завислих» записів журналу.
    pub recovered_pending: u32,
    /// Повернуто змін режиму гри, що лишилися з минулого запуску.
    pub restored_gamemode: RollbackResult,
    /// Помилка відновлення, якщо була.
    pub error: Option<String>,
}

/// Зміни, які можна запросити з інтерфейсу. Пріоритети процесів і служби — лише через
/// профіль режиму гри, напряму з UI їх змінити не можна.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "tweak", content = "value", rename_all = "snake_case")]
pub enum UiDesired {
    /// Схема живлення (GUID).
    PowerScheme(String),
    /// Ігровий режим Windows.
    GameMode(bool),
    /// Оверлейний режим живлення; `null` = збалансований.
    PowerMode(Option<String>),
    /// GPU для конкретного exe на гібридній системі.
    GpuPreference {
        /// Повний шлях до exe.
        exe_path: String,
        /// Бажаний GPU.
        preference: optimax_tweaks::catalog::GpuPref,
    },
    /// Програма в автозапуску.
    StartupApp {
        /// Звідки запускається.
        source: optimax_tweaks::startup::StartupSource,
        /// Ім'я запису.
        name: String,
        /// Бажаний стан.
        enabled: bool,
    },
}

impl From<UiDesired> for optimax_tweaks::Desired {
    fn from(d: UiDesired) -> Self {
        match d {
            UiDesired::PowerScheme(g) => Self::PowerScheme(g),
            UiDesired::GameMode(on) => Self::GameMode(on),
            UiDesired::PowerMode(guid) => Self::PowerMode(guid),
            UiDesired::GpuPreference {
                exe_path,
                preference,
            } => Self::GpuPreference {
                exe_path,
                preference,
            },
            UiDesired::StartupApp {
                source,
                name,
                enabled,
            } => Self::StartupApp {
                source,
                name,
                enabled,
            },
        }
    }
}

/// Нова версія на GitHub Releases.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AvailableUpdate {
    /// Версія.
    pub version: String,
    /// Що нового (текст із latest.json).
    pub notes: Option<String>,
    /// Те саме, розібране на розділи й пункти (показ без вставки HTML).
    pub sections: Vec<crate::release_notes::NoteSection>,
    /// Дата публікації (РРРР-ММ-ДД).
    pub date: Option<String>,
}

/// Результат перевірки оновлень.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UpdateStatus {
    /// Поточна версія.
    pub current: String,
    /// Новіша версія (`None` — встановлено останню).
    pub available: Option<AvailableUpdate>,
    /// Цю копію встановив інсталятор (лише її можна оновити з програми).
    pub installed: bool,
}

/// Прогрес завантаження оновлення.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UpdateProgress {
    /// Завантажено, байт.
    pub downloaded: u64,
    /// Розмір, байт (якщо сервер його повідомив).
    pub total: Option<u64>,
}

/// Чому перемикача автозапуску немає.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StartupLock {
    /// exe у теці Windows: системний компонент чи помічник драйвера — не вимикаємо.
    System,
    /// Прапорець StartupApproved має неочікуваний тип — не перезаписуємо.
    UnknownFlag,
}

/// Процеси запису автозапуску, що працюють зараз (виміряно під час показу списку).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct StartupRunning {
    /// Скільки процесів із цим exe.
    pub processes: u32,
    /// CPU, % від усього процесора.
    pub cpu_percent: f32,
    /// Пам'ять, МіБ.
    pub memory_mib: u64,
}

/// Запис автозапуску.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct StartupEntry {
    /// Звідки запускається.
    pub source: optimax_tweaks::startup::StartupSource,
    /// Ім'я запису (як у Диспетчері задач — ім'я значення Run чи файлу).
    pub name: String,
    /// Командний рядок (для теки — шлях до файлу).
    pub command: String,
    /// Шлях до exe (або до ярлика).
    pub exe: String,
    /// Чи файл exe існує (`None` — не перевіряли: ярлик або змінні середовища в шляху).
    pub exe_exists: Option<bool>,
    /// Увімкнено.
    pub enabled: bool,
    /// Чому не можна перемкнути (`None` — можна).
    pub lock: Option<StartupLock>,
    /// Поточне навантаження (`None` — не запущено або не з'ясовано).
    pub running: Option<StartupRunning>,
}

/// План із номером: застосувати можна лише план, який бекенд сам показав.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PlanHandle {
    /// Номер плану.
    pub id: u64,
    /// План.
    pub plan: Plan,
}

/// Результат застосування.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ApplyResult {
    /// Записи журналу.
    pub entry_ids: Vec<u64>,
    /// Точка відновлення, якщо створена саме зараз.
    pub restore_point: Option<RestorePointStatus>,
}

/// Залізо для екрана «Залізо».
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HardwareInfo {
    /// Модель CPU.
    pub cpu_name: String,
    /// Логічних ядер.
    pub logical_cores: u32,
    /// Фізичних ядер.
    pub physical_cores: Option<u32>,
    /// SMBIOS: пам'ять і BIOS.
    pub firmware: Option<FirmwareInfo>,
    /// Монітори.
    pub displays: Vec<DisplayInfo>,
    /// Відеокарти.
    pub gpus: Vec<GpuAdapter>,
}

/// Результат діагностики для UI.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DiagnosisResult {
    /// Діагноз.
    pub diagnosis: Diagnosis,
    /// Частота монітора, використана для аналізу.
    pub display_hz: Option<u32>,
    /// Повідомлення PresentMon.
    pub presentmon_messages: Vec<String>,
    /// Запис зупинено раніше.
    pub cancelled: bool,
    /// Зараховано чистої гри, с (без Alt+Tab, меню й прогріву).
    pub counted_s: f64,
    /// Чому запис завершився.
    pub end_reason: optimax_frametime::EndReason,
}

/// Знімок «Щойно лагнуло».
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LagCapture {
    /// Номер знімка в сесії.
    pub id: u32,
    /// Unix-час натискання, мс.
    pub at_unix_ms: Option<i64>,
    /// Аналіз вікна перед натисканням.
    pub result: DiagnosisResult,
}

/// Стан «Щойно лагнуло» для UI.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LagView {
    /// Гра.
    pub game: String,
    /// Скільки останніх секунд зберігає знімок.
    pub window_s: u32,
    /// Стеження йде.
    pub watching: bool,
    /// Останній стан буфера.
    pub status: Option<optimax_frametime::watch::WatchStatus>,
    /// Знімки, новіші першими.
    pub captures: Vec<LagCapture>,
    /// Останній знімок не вдався (скидається після вдалого).
    pub last_failure: Option<optimax_frametime::watch::CaptureFailure>,
    /// Чому стеження завершилось (якщо завершилось).
    pub ended: Option<optimax_frametime::watch::WatchEnd>,
}

/// Стан «Чому лагає?» на момент відкриття сторінки.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DiagnosisState {
    /// Запис іде просто зараз (результат прийде подією `diag-done`).
    pub recording: bool,
    /// Останній результат цього запуску програми.
    pub last: Option<DiagnosisResult>,
}

/// Процес для вибору гри.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ProcessEntry {
    /// Ім'я exe.
    pub name: String,
    /// CPU, % усього процесора (за останню секунду).
    pub cpu_percent: f32,
    /// Пам'ять, МіБ.
    pub memory_mib: u64,
}

/// Подія режиму гри.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameModeEvent {
    /// Стежник запущено.
    Watching {
        /// Профілі, за якими стежимо.
        games: Vec<String>,
    },
    /// Гра запустилась і режим гри ввімкнено.
    Started {
        /// Гра.
        game: String,
        /// PID.
        pid: u32,
        /// Застосовано.
        applied: Vec<Text>,
        /// Пропущено.
        skipped: Vec<Text>,
    },
    /// Точку відновлення не створено — потрібна згода користувача.
    NeedsConfirmation {
        /// Гра.
        game: String,
        /// Причина.
        reason: String,
    },
    /// Гру закрито, усе повернуто.
    Ended {
        /// Гра.
        game: String,
        /// Результат відкату.
        result: RollbackResult,
    },
    /// Стежник зупинено.
    Stopped,
}

/// Стан A/B-тесту для UI.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AbView {
    /// Гра.
    pub game: String,
    /// Опис зміни.
    pub plan: Plan,
    /// Тривалість прогону, с.
    pub seconds: u32,
    /// Спосіб старту прогонів.
    pub start: optimax_frametime::StartMode,
    /// Оцінка за виконаними прогонами (A — без зміни, B — зі зміною; наступний варіант,
    /// чи вже досить).
    pub report: optimax_experiment::Report,
    /// Найбільше прогонів на варіант.
    pub max_per_arm: u32,
    /// Перевіряється профіль гри цілком (а не один твік).
    pub profile: bool,
}

/// Стан тесту чутливості до роздільності для UI.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ResView {
    /// Гра.
    pub game: String,
    /// Тривалість прогону, с.
    pub seconds: u32,
    /// Спосіб старту прогонів.
    pub start: optimax_frametime::StartMode,
    /// Виконані прогони.
    pub runs: Vec<optimax_diagnosis::resolution::ResolutionRun>,
    /// Наступний прогін: `false` — рідна роздільність, `true` — знижена (`None` — досить).
    pub next_reduced: Option<bool>,
    /// Найбільше прогонів на кожну роздільність.
    pub max_per_arm: u32,
    /// Оцінка рушія порівнянь (інтервал, скільки ще прогонів).
    pub estimate: optimax_experiment::Report,
    /// Іде прогін.
    pub running: bool,
    /// Підсумок (коли висновок певний).
    pub report: Option<optimax_diagnosis::resolution::ResolutionReport>,
}

/// Підсумок A/B.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AbFinal {
    /// Звіт (A — без зміни, B — зі зміною).
    pub report: optimax_experiment::Report,
    /// Звіт Markdown для форуму.
    pub markdown: String,
    /// Чи залишено зміну.
    pub kept: bool,
}

/// Схеми живлення.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PowerSchemes {
    /// Усі схеми.
    pub schemes: Vec<optimax_core::power::PowerScheme>,
    /// GUID активної.
    pub active: String,
}

/// Профіль після перевірки безпеки + попередження.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ProfilePreview {
    /// Що буде збережено й застосовано.
    pub profile: optimax_gamemode::GameProfile,
    /// Що виправлено й чому.
    pub warnings: Vec<Text>,
}

/// Етап порівняння варіантів.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ExpPhase {
    /// Прогрів: гра йде, чекаємо сталої температури GPU.
    Warmup,
    /// Чекає наступного прогону.
    Ready,
    /// Іде прогін.
    Running,
    /// Висновок певний або прогонів досить.
    Done,
}

/// Хід прогріву.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ExpWarmup {
    /// Минуло, с.
    pub elapsed_s: f64,
    /// Найдовше, с.
    pub max_s: f64,
    /// Температура й нахил (немає температури — прогрів за часом).
    pub thermal: optimax_experiment::thermal::Thermal,
    /// Температура GPU доступна.
    pub has_temp: bool,
}

/// Стан порівняння варіантів для UI.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ExpView {
    /// Гра.
    pub game: String,
    /// Назви варіантів A, B… (від користувача; A — базовий).
    pub names: Vec<String>,
    /// Тривалість прогону, с.
    pub seconds: u32,
    /// Спосіб старту прогонів.
    pub start: optimax_frametime::StartMode,
    /// Етап.
    pub phase: ExpPhase,
    /// Прогрів (поки триває або якщо був).
    pub warmup: Option<ExpWarmup>,
    /// Оцінка за поточними прогонами.
    pub report: optimax_experiment::Report,
    /// Найбільше прогонів на варіант.
    pub max_per_arm: u32,
    /// Ідентифікатор збереженого результату (після завершення).
    pub saved_id: Option<String>,
}

/// Збережене порівняння.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ExpRecord {
    /// Ідентифікатор (ім'я файлу).
    pub id: String,
    /// Гра.
    pub game: String,
    /// Коли завершено, Unix-мс.
    pub at_unix_ms: i64,
    /// Назви варіантів A, B….
    pub names: Vec<String>,
    /// Тривалість прогону, с.
    pub seconds: u32,
    /// Підсумок.
    pub report: optimax_experiment::Report,
}
