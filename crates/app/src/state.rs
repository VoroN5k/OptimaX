//! Стан застосунку: рушій змін, сесії, налаштування.

use crate::settings::Settings;
use optimax_core::memory::WinMemory;
use optimax_core::power::WinPower;
use optimax_core::process::WinProcesses;
use optimax_core::registry::WinRegistry;
use optimax_core::service::WinServices;
use optimax_frametime::{BundledPresentMon, CancelToken};
use optimax_metrics::Sampler;
use optimax_tweaks::restore::WinRestorePoint;
use optimax_tweaks::{Confirm, Engine, FileStore, Plan};
use std::collections::HashMap;
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
    /// Рушій змін (спільний для UI і режиму гри).
    pub engine: Arc<Mutex<Engine<'static>>>,
    /// Плани, показані користувачу: застосувати можна лише план із цього списку.
    pub plans: Mutex<HashMap<u64, Plan>>,
    /// Лічильник номерів планів.
    pub next_plan_id: Mutex<u64>,
    /// Живі метрики для UI.
    pub metrics: Mutex<Option<Sampler>>,
    /// Поточний запис діагностики.
    pub recording: Mutex<Option<CancelToken>>,
    /// A/B-сесія.
    pub ab: Mutex<Option<crate::commands::ab::AbSession>>,
    /// Режим гри.
    pub gamemode: Mutex<Option<crate::commands::gamemode::Watcher>>,
    /// Користувач дозволив режиму гри працювати без точки відновлення (до кінця сеансу).
    pub gamemode_allow_without_restore: Arc<Mutex<bool>>,
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
}

impl AppState {
    /// Створює стан. Журнал змін — `%LOCALAPPDATA%\OptimaX\journal.json`.
    pub fn new() -> anyhow::Result<Self> {
        let store =
            FileStore::default_location().ok_or_else(|| anyhow::anyhow!("немає LOCALAPPDATA"))?;
        // Сховище журналу живе весь час роботи програми — свідомо «відпускаємо» один раз.
        let store: &'static FileStore = Box::leak(Box::new(store));
        let engine = Engine::new(&REGISTRY, &POWER, &RESTORE, store)?
            .with_processes(&PROCESSES)
            .with_services(&SERVICES);
        Ok(Self {
            engine: Arc::new(Mutex::new(engine)),
            plans: Mutex::new(HashMap::new()),
            next_plan_id: Mutex::new(1),
            metrics: Mutex::new(None),
            recording: Mutex::new(None),
            ab: Mutex::new(None),
            gamemode: Mutex::new(None),
            gamemode_allow_without_restore: Arc::new(Mutex::new(false)),
            settings: Mutex::new(Settings::load()),
            presentmon: OnceLock::new(),
            startup: Mutex::new(crate::dto::StartupReport::default()),
            upscale: Mutex::new(None),
            net_test: Mutex::new(None),
            cleanup: Mutex::new(Default::default()),
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
