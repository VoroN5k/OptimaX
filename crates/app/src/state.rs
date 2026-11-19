//! Стан застосунку: рушій змін, сесії, налаштування.

use crate::settings::Settings;
use optimax_core::memory::WinMemory;
use optimax_core::power::WinPower;
use optimax_core::process::WinProcesses;
use optimax_core::registry::WinRegistry;
use optimax_core::service::WinServices;
use optimax_frametime::{BundledPresentMon, CancelToken, RecordProgress};
use optimax_metrics::Sampler;
use optimax_tweaks::restore::WinRestorePoint;
use optimax_tweaks::{Confirm, Plan};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

/// Системні інтерфейси — порожні структури без стану, тому `static` безпечні й незмінні.
pub static REGISTRY: WinRegistry = WinRegistry;
/// Схеми живлення.
pub static POWER: WinPower = WinPower;
/// Точка відновлення.
pub static RESTORE: WinRestorePoint = WinRestorePoint;
/// Процеси.
pub static PROCESSES: WinProcesses = WinProcesses;
/// Служби.
pub static SERVICES: WinServices = WinServices;
/// Пам'ять.
pub static MEMORY: WinMemory = WinMemory;

/// Блокує м'ютекс, не панікуючи на «отруєному» стані (дані все одно консистентні: журнал на диску).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Підтвердження без діалогу: відповідь задана заздалегідь, причина відмови запам'ятовується,
/// щоб UI міг показати модальне вікно й повторити дію з дозволом.
pub struct FlagConfirm {
    allow: bool,
    reason: Mutex<Option<String>>,
}

impl FlagConfirm {
    /// Новий.
    pub fn new(allow: bool) -> Self {
        Self {
            allow,
            reason: Mutex::new(None),
        }
    }

    /// Причина, з якою точку відновлення не створено (якщо питання виникало).
    pub fn reason(&self) -> Option<String> {
        lock(&self.reason).clone()
    }
}

impl Confirm for FlagConfirm {
    fn proceed_without_restore_point(&self, reason: &str) -> bool {
        *lock(&self.reason) = Some(reason.to_string());
        self.allow
    }
}

/// Стан застосунку.
pub struct AppState {
    /// Зміни системи — через службу або рушій у процесі (спільні для UI, A/B і режиму гри).
    pub changes: Arc<Mutex<crate::changes::ChangeHub>>,
    /// Плани, показані користувачу: застосувати можна лише план із цього списку.
    pub plans: Mutex<HashMap<u64, Plan>>,
    /// Лічильник номерів планів.
    pub next_plan_id: Mutex<u64>,
    /// Живі метрики для UI.
    pub metrics: Mutex<Option<Sampler>>,
    /// Поточний запис діагностики.
    pub recording: Mutex<Option<CancelToken>>,
    /// Хід запису, що йде зараз (діагностика, A/B, тест роздільності) — для позначки в оверлеї.
    pub record_progress: Mutex<Option<RecordProgress>>,
    /// A/B-сесія.
    pub ab: Mutex<Option<crate::commands::ab::AbSession>>,
    /// Тест чутливості до роздільності.
    pub res_test: Mutex<Option<crate::commands::resolution::ResSession>>,
    /// Порівняння варіантів (науковий A/B).
    pub experiment: Mutex<Option<crate::commands::experiment::ExpSession>>,
    /// Тест стабільності, що йде.
    pub stability: Mutex<Option<crate::commands::stability::StabSession>>,
    /// «Щойно лагнуло»: стеження з кільцевим буфером.
    pub lag: Mutex<Option<crate::commands::lag::LagSession>>,
    /// Режим гри й чорна скринька (спільне з агентом у треї: `crate::session`).
    pub session: crate::session::SessionCore,
    /// Оверлей у грі (є, поки ввімкнений у налаштуваннях).
    pub overlay: Mutex<Option<crate::commands::overlay::Controller>>,
    /// Стежник режиму гри у вікні (коли агента в треї немає).
    pub gamemode: Mutex<Option<crate::session::gamemode::Watcher>>,
    /// Налаштування.
    pub settings: Mutex<Settings>,
    /// Розпакований PresentMon (один раз за запуск).
    pub presentmon: OnceLock<BundledPresentMon>,
    /// Що зроблено при старті (відновлення після збою).
    pub startup: Mutex<crate::dto::StartupReport>,
    /// Контролер апскейлу (запускається в `setup`, бо потребує дескриптора застосунку).
    pub upscale: Mutex<Option<crate::commands::upscale::Controller>>,
    /// Поточний замір мережі.
    pub net_test: Mutex<Option<optimax_network::CancelToken>>,
    /// Очищення диска: скан, план, точка відновлення сесії.
    pub cleanup: Mutex<crate::commands::cleanup::CleanupState>,
    /// Кеш головного екрана (аудит і скан диска — раз за запуск).
    pub dashboard: Mutex<crate::commands::dashboard::DashCache>,
    /// Останній результат діагностики (для звіту для відгуку).
    pub last_diagnosis: Mutex<Option<crate::dto::DiagnosisResult>>,
    /// Id останнього результату в «Історії» (`last_diagnosis` збережено під ним) — для «було».
    pub last_record_id: Mutex<Option<String>>,
    /// Остання перевірка «було / стало» (гра й зв'язок): повторні «Я зробив» з тим самим «було».
    /// (гра, обрана порада, зв'язок).
    pub last_check: Mutex<
        Option<(
            String,
            Option<optimax_diagnosis::CheckChoice>,
            optimax_history::CheckLink,
        )>,
    >,
    /// Останні рядки журналу дій (для звіту для відгуку), не більше [`LOG_TAIL`].
    pub log_tail: Mutex<VecDeque<crate::log_layer::LogLine>>,
    /// Знімок процесів для вибору гри: CPU рахується за час від попереднього запиту
    /// (UI питає раз на кілька секунд), а не за коротке вікно — цифри стабільні.
    pub processes: Mutex<Option<sysinfo::System>>,
    /// Бібліотека ігор: кеш скану й поточне визначення процесу гри.
    pub library: Mutex<crate::commands::library::LibraryState>,
    /// Шлях до привілейованих даних: служба або сам процес (спільний для потоків).
    pub link: Arc<crate::link::Link>,
}

/// Скільки останніх рядків журналу дій пам'ятаємо для звіту.
pub const LOG_TAIL: usize = 1000;

impl AppState {
    /// Створює стан. Журнал змін — `journal.json` у захищеній теці даних (відкривається при
    /// першій зміні: службою або рушієм у процесі).
    pub fn new() -> anyhow::Result<Self> {
        let link = Arc::new(crate::link::Link::new());
        let changes = Arc::new(Mutex::new(crate::changes::ChangeHub::new(link.clone())));
        Ok(Self {
            session: crate::session::SessionCore::new(link.clone(), changes.clone()),
            changes,
            plans: Mutex::new(HashMap::new()),
            next_plan_id: Mutex::new(1),
            metrics: Mutex::new(None),
            recording: Mutex::new(None),
            record_progress: Mutex::new(None),
            ab: Mutex::new(None),
            res_test: Mutex::new(None),
            experiment: Mutex::new(None),
            stability: Mutex::new(None),
            lag: Mutex::new(None),
            overlay: Mutex::new(None),
            gamemode: Mutex::new(None),
            settings: Mutex::new(Settings::load()),
            presentmon: OnceLock::new(),
            startup: Mutex::new(crate::dto::StartupReport::default()),
            upscale: Mutex::new(None),
            net_test: Mutex::new(None),
            cleanup: Mutex::new(Default::default()),
            dashboard: Mutex::new(Default::default()),
            last_diagnosis: Mutex::new(None),
            last_record_id: Mutex::new(None),
            last_check: Mutex::new(None),
            log_tail: Mutex::new(VecDeque::with_capacity(LOG_TAIL)),
            processes: Mutex::new(None),
            library: Mutex::new(Default::default()),
            link,
        })
    }

    /// Вбудований PresentMon (розпаковується при першому зверненні, хеш перевіряється щоразу при запуску).
    pub fn presentmon(&self) -> Result<&BundledPresentMon, optimax_frametime::FrametimeError> {
        if let Some(pm) = self.presentmon.get() {
            return Ok(pm);
        }
        let pm = BundledPresentMon::install_default()?;
        Ok(self.presentmon.get_or_init(|| pm))
    }
}
