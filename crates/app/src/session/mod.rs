//! Режим гри й чорна скринька без Tauri (етап 4б `docs/SERVICE.md`). Їх веде або вікно програми,
//! або агент у треї — той, хто реалізує [`Host`]. Якщо агент працює, веде він; вікно тоді лише
//! показує його стан (`crate::agent_link`).

pub mod blackbox;
pub mod gamemode;

use crate::changes::ChangeHub;
use crate::dto::GameModeEvent;
use crate::error::CmdResult;
use crate::link::{Link, WatchHandle};
use optimax_frametime::watch::{WatchEvent, WatchOptions};
use optimax_metrics::CpuTempSource;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// Спільний стан сесії: шлях до служби, рушій змін, чорна скринька.
pub struct SessionCore {
    /// Служба або сам процес.
    pub link: Arc<Link>,
    /// Зміни системи (спільні з рештою програми).
    pub changes: Arc<Mutex<ChangeHub>>,
    /// Користувач дозволив режиму гри працювати без точки відновлення (до кінця запуску).
    pub allow_without_restore: Mutex<bool>,
    /// Чорна скринька, що йде.
    pub blackbox: Mutex<Option<blackbox::BlackBox>>,
    /// Лічильник сесій чорної скриньки.
    pub blackbox_seq: Mutex<u64>,
}

impl SessionCore {
    /// Новий стан.
    pub fn new(link: Arc<Link>, changes: Arc<Mutex<ChangeHub>>) -> Self {
        Self {
            link,
            changes,
            allow_without_restore: Mutex::new(false),
            blackbox: Mutex::new(None),
            blackbox_seq: Mutex::new(0),
        }
    }
}

/// Хто веде сесію: вікно програми або агент у треї.
pub trait Host: Send + Sync + 'static {
    /// Спільний стан.
    fn core(&self) -> &SessionCore;
    /// Іде інший запис гри (тоді чорна скринька не стежить).
    fn busy(&self) -> bool;
    /// Джерело температури CPU для стеження (якщо дозволено).
    fn cpu_temp(&self) -> Option<CpuTempSource>;
    /// Починає стеження «Щойно лагнуло» для чорної скриньки.
    fn start_watch(&self, options: WatchOptions) -> CmdResult<(WatchHandle, Receiver<WatchEvent>)>;
    /// Подія режиму гри.
    fn gamemode_event(&self, event: GameModeEvent);
    /// Стан чорної скриньки змінився (`blackbox::view`).
    fn blackbox_changed(&self);
}

/// Спільне посилання на [`Host`] для потоків.
pub type HostRef = Arc<dyn Host>;
