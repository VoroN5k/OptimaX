//! Режим гри й чорна скринька у вікні програми: [`TauriHost`] для `crate::session` і міст до
//! агента в треї ([`watch_agent`]) — коли агент веде режим гри, його стан перетворюється на ті
//! самі події `gamemode` / `blackbox`, тож інтерфейс не знає, хто саме веде.

use crate::agent_link::{self, AgentStatus};
use crate::dto::GameModeEvent;
use crate::error::CmdResult;
use crate::link::WatchHandle;
use crate::session::{blackbox, Host, HostRef, SessionCore};
use crate::state::{lock, AppState};
use optimax_frametime::watch::{WatchEvent, WatchOptions};
use optimax_metrics::CpuTempSource;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Як часто вікно перечитує стан агента.
const AGENT_POLL: Duration = Duration::from_millis(500);

/// Сесію веде вікно програми.
pub struct TauriHost(AppHandle);

impl TauriHost {
    fn state(&self) -> &AppState {
        self.0.state::<AppState>().inner()
    }
}

/// Іде інший запис гри (тоді чорна скринька не стежить).
pub fn busy(state: &AppState) -> bool {
    lock(&state.recording).is_some()
        || lock(&state.ab).is_some()
        || super::resolution::is_running(state)
        || super::experiment::is_active(state)
        || super::stability::is_active(state)
        || super::lag::is_watching(state)
}

impl Host for TauriHost {
    fn core(&self) -> &SessionCore {
        &self.state().session
    }

    fn busy(&self) -> bool {
        busy(self.state())
    }

    fn cpu_temp(&self) -> Option<CpuTempSource> {
        super::sensors::cpu_temp_source(self.state())
    }

    fn start_watch(&self, options: WatchOptions) -> CmdResult<(WatchHandle, Receiver<WatchEvent>)> {
        let state = self.state();
        state.link.start_watch(Some(state), options)
    }

    fn gamemode_event(&self, event: GameModeEvent) {
        let _ = self.0.emit("gamemode", event);
    }

    fn blackbox_changed(&self) {
        let _ = self
            .0
            .emit("blackbox", blackbox::view(&self.state().session));
    }
}

/// [`Host`] вікна програми.
pub fn host(app: &AppHandle) -> HostRef {
    Arc::new(TauriHost(app.clone()))
}

/// Потік вікна: поки агент веде режим гри, перетворює зміни його стану на події інтерфейсу. Якщо
/// агент з'явився, а вікно само стежило, — віддає стеження агенту.
pub fn watch_agent(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("optimax-agent-link".into())
        .spawn(move || {
            let mut shown: Option<AgentStatus> = None;
            loop {
                std::thread::sleep(AGENT_POLL);
                if !agent_link::agent_active() {
                    // Агент зник — те, що показувалось від нього, уже неправда.
                    if shown.take().is_some_and(|s| s.watching) {
                        let _ = app.emit("gamemode", GameModeEvent::Stopped);
                    }
                    continue;
                }
                let state = app.state::<AppState>();
                if let Some(w) = lock(&state.gamemode).take() {
                    tracing::info!("режим гри передано агенту в треї");
                    w.shutdown();
                    if let Err(e) = agent_link::update_config(|c| c.gamemode = true) {
                        tracing::warn!("агент у треї: налаштування не записано: {e}");
                    }
                }
                let Some(status) = agent_link::read_status() else {
                    continue;
                };
                if shown.as_ref().is_some_and(|s| s.seq == status.seq) {
                    continue;
                }
                let (events, blackbox) = agent_link::events_between(shown.as_ref(), &status);
                for e in events {
                    let _ = app.emit("gamemode", e);
                }
                if blackbox {
                    let _ = app.emit("blackbox", &status.blackbox);
                }
                shown = Some(status);
            }
        });
    if let Err(e) = spawned {
        tracing::error!("міст до агента в треї не запущено: {e}");
    }
}
