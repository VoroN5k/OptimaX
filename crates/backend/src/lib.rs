//! Бекенд OptimaX без Tauri (етап 2 з `docs/SERVICE.md`): те, що потребує прав адміністратора й
//! тому виконується в службі, а без служби — у самій програмі.
//!
//! Датчики CPU через PawnIO ([`sensors`]), запис гри ([`record`]), стеження й оверлей ([`watch`]),
//! зміни системи з журналом і відкатом ([`changes`]).

#![forbid(unsafe_code)]

pub mod changes;
pub mod record;
pub mod sensors;
pub mod watch;

#[cfg(windows)]
pub use self::service::handler;

#[cfg(windows)]
mod service {
    use crate::changes::ChangesHost;
    use crate::record::{RecordSpec, Recorder};
    use crate::sensors::SensorCache;
    use crate::watch::{LiveHost, WatchHost, WatchSpec};
    use optimax_frametime::live::LiveMonitor;
    use optimax_frametime::watch::Watcher;
    use optimax_frametime::{record_in_session, BundledPresentMon, FrametimeError};
    use optimax_metrics::{CpuTempSource, GpuApi, NvmlGpu, SysinfoSource};
    use optimax_service::host::{Handler, Peer};
    use optimax_service::protocol::{Command, Reply};
    use std::sync::{Arc, OnceLock};

    fn json<T: serde::Serialize>(v: &T) -> Reply {
        serde_json::to_value(v).map_or_else(|e| Reply::Error(e.to_string()), Reply::Json)
    }

    /// Стан служби: відкриті датчики, запис, розпакований PresentMon.
    struct Backend {
        sensors: Arc<SensorCache>,
        recorder: Recorder,
        watch: WatchHost,
        live: LiveHost,
        changes: ChangesHost,
        presentmon: Arc<OnceLock<BundledPresentMon>>,
    }

    /// Вбудований PresentMon (розпаковується й перевіряється за SHA-256 один раз).
    fn presentmon(
        cell: &OnceLock<BundledPresentMon>,
    ) -> Result<&BundledPresentMon, FrametimeError> {
        if let Some(p) = cell.get() {
            return Ok(p);
        }
        let installed = BundledPresentMon::install_default()?;
        Ok(cell.get_or_init(|| installed))
    }

    fn gpu() -> Option<Box<dyn GpuApi>> {
        NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>)
    }

    impl Backend {
        fn temp_source(&self, on: bool) -> Option<CpuTempSource> {
            on.then(|| {
                let s = self.sensors.clone();
                CpuTempSource(Arc::new(move || s.temp()))
            })
        }

        fn watch_start(&self, spec: &serde_json::Value) -> Reply {
            let spec: WatchSpec = match serde_json::from_value(spec.clone()) {
                Ok(s) => s,
                Err(e) => return Reply::Error(format!("некоректний запит стеження: {e}")),
            };
            let temp = self.temp_source(spec.cpu_temp);
            let pm = match presentmon(&self.presentmon) {
                Ok(p) => p,
                Err(e) => return Reply::Error(e.to_string()),
            };
            tracing::info!("служба: стеження за {:?}", spec.target);
            let started = self.watch.start(&spec, |mut options, session| {
                options.sampler.cpu_temp = temp;
                Watcher::start_in_session(
                    pm,
                    options,
                    Box::new(SysinfoSource::new()),
                    gpu(),
                    session,
                )
            });
            started.map_or_else(Reply::Error, |()| Reply::Json(serde_json::Value::Null))
        }

        fn live_start(&self) -> Reply {
            let pm = match presentmon(&self.presentmon) {
                Ok(p) => p,
                Err(e) => return Reply::Error(e.to_string()),
            };
            let started = self
                .live
                .start(|fg| LiveMonitor::start_with(pm, Box::new(SysinfoSource::new()), gpu(), fg));
            started.map_or_else(Reply::Error, |()| Reply::Json(serde_json::Value::Null))
        }

        fn record_start(&self, spec: &serde_json::Value) -> Reply {
            let spec: RecordSpec = match serde_json::from_value(spec.clone()) {
                Ok(s) => s,
                Err(e) => return Reply::Error(format!("некоректний запит запису: {e}")),
            };
            let temp = self.temp_source(spec.cpu_temp);
            let pm = self.presentmon.clone();
            let started =
                self.recorder
                    .start(spec, move |options, cancel, session, on_progress| {
                        // PresentMon — той самий вбудований, з перевіркою SHA-256 (CLAUDE.md 2.1).
                        let pm = presentmon(&pm)?;
                        let mut options = options.clone();
                        options.sampler.cpu_temp = temp;
                        tracing::info!("служба: запис гри {:?}", options.target);
                        let r = record_in_session(
                            pm,
                            &options,
                            Box::new(SysinfoSource::new()),
                            gpu(),
                            cancel,
                            session,
                            on_progress,
                        );
                        if let Err(e) = &r {
                            tracing::warn!("служба: запис не вдався: {e}");
                        }
                        r
                    });
            match started {
                Ok(()) => Reply::Json(serde_json::Value::Null),
                Err(e) => Reply::Error(e),
            }
        }
    }

    /// Виконавець команд служби. Модулі PawnIO відкриваються при першому запиті й тримаються
    /// відкритими весь час роботи служби.
    pub fn handler() -> Handler {
        let b = Backend {
            sensors: Arc::new(SensorCache::new()),
            recorder: Recorder::new(),
            watch: WatchHost::new(),
            live: LiveHost::new(),
            changes: ChangesHost::new(),
            presentmon: Arc::new(OnceLock::new()),
        };
        Arc::new(move |peer: &Peer, cmd: &Command| {
            // Зміни системи — від імені того, хто під'єднався (його журнал і гілка HKCU).
            if let Some(r) = b.changes.handle(peer, cmd) {
                return r;
            }
            other(&b, cmd)
        })
    }

    fn other(b: &Backend, cmd: &Command) -> Reply {
        match cmd {
            Command::Sensors => json(&b.sensors.read()),
            Command::CpuTemp => json(&b.sensors.temp()),
            Command::RecordStart { spec } => b.record_start(spec),
            Command::RecordPoll {
                foreground_pid,
                hotkey,
            } => b
                .recorder
                .poll(*foreground_pid, *hotkey)
                .map_or_else(Reply::Error, |r| json(&r)),
            Command::RecordCancel => {
                b.recorder.cancel();
                Reply::Json(serde_json::Value::Null)
            }
            Command::WatchStart { spec } => b.watch_start(spec),
            Command::WatchPoll {
                foreground_pid,
                hotkey,
                capture,
            } => b
                .watch
                .poll(*foreground_pid, *hotkey, *capture)
                .map_or_else(Reply::Error, |e| json(&e)),
            Command::WatchStop => {
                b.watch.stop();
                Reply::Json(serde_json::Value::Null)
            }
            Command::LiveStart => b.live_start(),
            Command::LivePoll { foreground_pid } => b
                .live
                .poll(*foreground_pid)
                .map_or_else(Reply::Error, |s| json(&s)),
            Command::LiveStop => {
                b.live.stop();
                Reply::Json(serde_json::Value::Null)
            }
            // `ping` служба відповідає сама.
            Command::Ping => Reply::Error("ping обробляє служба".into()),
            // Зміни обробляє `ChangesHost` вище.
            Command::TweakPlan { .. }
            | Command::TweakApply { .. }
            | Command::PurgeStandby
            | Command::JournalEntries
            | Command::JournalRollback { .. }
            | Command::JournalRollbackAll
            | Command::JournalRecover => Reply::Error("журнал змін недоступний".into()),
        }
    }
}
