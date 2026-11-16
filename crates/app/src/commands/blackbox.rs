//! Чорна скринька у вікні програми: стан для інтерфейсу й пауза на час іншого запису гри. Сама
//! логіка — у `crate::session::blackbox`; якщо режим гри веде агент у треї, стан береться від
//! нього, а пауза передається йому файлом (`crate::agent_link`).

use crate::agent_link;
use crate::error::CmdResult;
use crate::session::blackbox::{self as core_bb, BlackBoxView};
use crate::session::HostRef;
use crate::state::AppState;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// Скільки чекати, поки агент поставить скриньку на паузу.
const AGENT_PAUSE_WAIT: Duration = Duration::from_secs(5);

/// Скільки пауз агента тримає цей процес (позначка одна на процес).
static AGENT_PAUSES: AtomicUsize = AtomicUsize::new(0);

/// Пауза скриньки на час іншого запису гри (у вікні й, якщо працює, в агенті). Знімається при
/// знищенні.
pub struct Pause {
    _local: core_bb::Pause,
    agent: bool,
}

impl Pause {
    /// Ставить на паузу (блокує до зупинки стеження). Без сесії — нічого не робить.
    pub fn new(app: &AppHandle) -> Self {
        let host: HostRef = super::session::host(app);
        let local = core_bb::Pause::new(&host);
        let agent = agent_link::agent_active();
        if agent {
            if AGENT_PAUSES.fetch_add(1, Ordering::SeqCst) == 0 {
                agent_link::set_pause(true);
            }
            wait_agent_paused();
        }
        Self {
            _local: local,
            agent,
        }
    }
}

/// Чекає, поки чорна скринька агента зупинить стеження (або її немає).
fn wait_agent_paused() {
    let deadline = Instant::now() + AGENT_PAUSE_WAIT;
    while Instant::now() < deadline {
        let paused = agent_link::read_status()
            .is_none_or(|s| s.blackbox.active.as_ref().is_none_or(|a| a.paused));
        if paused {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    tracing::warn!("агент у треї не поставив чорну скриньку на паузу вчасно");
}

impl Drop for Pause {
    fn drop(&mut self) {
        if self.agent && AGENT_PAUSES.fetch_sub(1, Ordering::SeqCst) == 1 {
            agent_link::set_pause(false);
        }
    }
}

/// Стан чорної скриньки: поточна сесія й остання збережена.
#[tauri::command]
pub async fn blackbox_state(app: AppHandle) -> CmdResult<BlackBoxView> {
    super::blocking(move || {
        if agent_link::agent_active() {
            if let Some(s) = agent_link::read_status() {
                return Ok(s.blackbox);
            }
        }
        Ok(core_bb::view(&app.state::<AppState>().session))
    })
    .await
}
