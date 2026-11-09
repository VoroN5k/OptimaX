//! Бекенд OptimaX без Tauri (етап 2 з `docs/SERVICE.md`): те, що потребує прав адміністратора й
//! тому виконується в службі, а без служби — у самій програмі.
//!
//! Датчики CPU через PawnIO ([`sensors`]) і запис гри ([`record`]).

#![forbid(unsafe_code)]

pub mod record;
pub mod sensors;

#[cfg(windows)]
pub use self::service::handler;

#[cfg(windows)]
mod service {
    use crate::record::{RecordSpec, Recorder};
    use crate::sensors::SensorCache;
    use optimax_frametime::{record_in_session, BundledPresentMon};
    use optimax_metrics::{CpuTempSource, GpuApi, NvmlGpu, SysinfoSource};
    use optimax_service::host::Handler;
    use optimax_service::protocol::{Command, Reply};
    use std::sync::{Arc, OnceLock};

    fn json<T: serde::Serialize>(v: &T) -> Reply {
        serde_json::to_value(v).map_or_else(|e| Reply::Error(e.to_string()), Reply::Json)
    }

    /// Стан служби: відкриті датчики, запис, розпакований PresentMon.
    struct Backend {
        sensors: Arc<SensorCache>,
        recorder: Recorder,
        presentmon: Arc<OnceLock<BundledPresentMon>>,
    }

    impl Backend {
        fn record_start(&self, spec: &serde_json::Value) -> Reply {
            let spec: RecordSpec = match serde_json::from_value(spec.clone()) {
                Ok(s) => s,
                Err(e) => return Reply::Error(format!("некоректний запит запису: {e}")),
            };
            let temp = spec.cpu_temp.then(|| {
                let s = self.sensors.clone();
                CpuTempSource(Arc::new(move || s.temp()))
            });
            let pm = self.presentmon.clone();
            let started =
                self.recorder
                    .start(spec, move |options, cancel, session, on_progress| {
                        // PresentMon — той самий вбудований, з перевіркою SHA-256 (CLAUDE.md 2.1).
                        let pm = match pm.get() {
                            Some(p) => p,
                            None => {
                                let installed = BundledPresentMon::install_default()?;
                                pm.get_or_init(|| installed)
                            }
                        };
                        let mut options = options.clone();
                        options.sampler.cpu_temp = temp;
                        let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
                        tracing::info!("служба: запис гри {:?}", options.target);
                        let r = record_in_session(
                            pm,
                            &options,
                            Box::new(SysinfoSource::new()),
                            gpu,
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
            presentmon: Arc::new(OnceLock::new()),
        };
        Arc::new(move |cmd: &Command| match cmd {
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
            // `ping` служба відповідає сама.
            Command::Ping => Reply::Error("ping обробляє служба".into()),
        })
    }
}
