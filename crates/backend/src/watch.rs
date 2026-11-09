//! «Щойно лагнуло?», чорна скринька й оверлей у службі: PresentMon, ETW і метрики — тут;
//! активне вікно, гарячі клавіші, звук і повідомлення — в інтерфейсі, що опитує службу.
//!
//! Стеження й оверлей — по одному одночасно. Інтерфейс не опитує понад [`ABANDON`] — зупиняються.

use crate::record::{lock, Inputs, ABANDON};
use optimax_frametime::live::{Foreground, LiveMonitor, LiveStats};
use optimax_frametime::watch::{
    CaptureFailure, FreezeMark, WatchEnd, WatchEvent, WatchNotice, WatchOptions, WatchSession,
    WatchStatus, Watcher,
};
use optimax_frametime::{FrametimeError, Recording, Target};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Що стежити (передається каналом; клавіша, звук і повідомлення — в інтерфейсі).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchSpec {
    /// Гра.
    pub target: Target,
    /// Скільки останніх секунд зберігає знімок, мс.
    pub window_ms: u64,
    /// Чорна скринька: поріг фризу, мс.
    pub freeze_ms: Option<f64>,
    /// Читати температуру CPU.
    pub cpu_temp: bool,
}

impl WatchSpec {
    /// З параметрів стеження інтерфейсу.
    pub fn from_options(o: &WatchOptions) -> Self {
        Self {
            target: o.target.clone(),
            window_ms: u64::try_from(o.window.as_millis()).unwrap_or(u64::MAX),
            freeze_ms: o.freeze_ms,
            cpu_temp: o.sampler.cpu_temp.is_some(),
        }
    }

    /// Параметри в службі: без клавіші, звуку й повідомлень.
    pub fn options(&self) -> WatchOptions {
        let mut o = WatchOptions::new(self.target.clone());
        o.window = Duration::from_millis(self.window_ms);
        o.freeze_ms = self.freeze_ms;
        o.hotkey = false;
        o.sound = false;
        o.notices = None;
        o
    }
}

/// Подія стеження в каналі (та сама [`WatchEvent`] плюс повідомлення для сесії користувача).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireWatchEvent {
    /// Стан.
    Status {
        /// Стан.
        status: WatchStatus,
    },
    /// Натиснуто.
    Pressed,
    /// Знімок.
    Captured {
        /// Запис вікна.
        recording: Box<Recording>,
        /// Unix-час натискання, мс.
        at_unix_ms: Option<i64>,
    },
    /// Знімок не вдався.
    CaptureFailed {
        /// Причина.
        reason: CaptureFailure,
        /// Unix-час натискання, мс.
        at_unix_ms: Option<i64>,
    },
    /// Чорна скринька.
    Freezes {
        /// Знімок.
        recording: Option<Box<Recording>>,
        /// Фризи.
        freezes: Vec<FreezeMark>,
        /// Чому знімка немає.
        failure: Option<CaptureFailure>,
    },
    /// Кінець.
    Ended {
        /// Чому.
        end: WatchEnd,
    },
    /// Повідомлення для сесії користувача.
    Notice {
        /// Яке.
        notice: WatchNotice,
    },
}

impl From<WatchEvent> for WireWatchEvent {
    fn from(e: WatchEvent) -> Self {
        match e {
            WatchEvent::Status(status) => Self::Status { status },
            WatchEvent::Pressed => Self::Pressed,
            WatchEvent::Captured {
                recording,
                at_unix_ms,
            } => Self::Captured {
                recording,
                at_unix_ms,
            },
            WatchEvent::CaptureFailed { reason, at_unix_ms } => {
                Self::CaptureFailed { reason, at_unix_ms }
            }
            WatchEvent::Freezes {
                recording,
                freezes,
                failure,
            } => Self::Freezes {
                recording,
                freezes,
                failure,
            },
            WatchEvent::Ended(end) => Self::Ended { end },
        }
    }
}

impl WireWatchEvent {
    /// Назад у подію стеження; повідомлення — окремо (`Err`).
    pub fn into_event(self) -> Result<WatchEvent, WatchNotice> {
        Ok(match self {
            Self::Status { status } => WatchEvent::Status(status),
            Self::Pressed => WatchEvent::Pressed,
            Self::Captured {
                recording,
                at_unix_ms,
            } => WatchEvent::Captured {
                recording,
                at_unix_ms,
            },
            Self::CaptureFailed { reason, at_unix_ms } => {
                WatchEvent::CaptureFailed { reason, at_unix_ms }
            }
            Self::Freezes {
                recording,
                freezes,
                failure,
            } => WatchEvent::Freezes {
                recording,
                freezes,
                failure,
            },
            Self::Ended { end } => WatchEvent::Ended(end),
            Self::Notice { notice } => return Err(notice),
        })
    }
}

/// Сесія користувача, яку передає інтерфейс.
struct RemoteWatchSession {
    inputs: Arc<Inputs>,
    queue: Arc<Mutex<Vec<WireWatchEvent>>>,
}

impl WatchSession for RemoteWatchSession {
    fn foreground_pid(&self) -> Option<u32> {
        self.inputs.foreground()
    }

    fn hotkey_pressed(&self) -> bool {
        self.inputs.take_hotkey()
    }

    fn notice(&self, notice: WatchNotice) {
        lock(&self.queue).push(WireWatchEvent::Notice { notice });
    }
}

struct ActiveWatch {
    inputs: Arc<Inputs>,
    queue: Arc<Mutex<Vec<WireWatchEvent>>>,
    watcher: Arc<Mutex<Option<Watcher>>>,
    ended: Arc<AtomicBool>,
}

/// Стеження в службі (одне одночасно).
#[derive(Default)]
pub struct WatchHost {
    active: Mutex<Option<ActiveWatch>>,
}

impl std::fmt::Debug for WatchHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WatchHost")
    }
}

/// Скільки чекати подію, перш ніж перевірити, чи інтерфейс ще живий.
const PUMP_TICK: Duration = Duration::from_secs(1);

impl WatchHost {
    /// Нічого не запускає.
    pub fn new() -> Self {
        Self::default()
    }

    /// Починає стеження. `run` запускає [`Watcher`] із наданою сесією.
    pub fn start(
        &self,
        spec: &WatchSpec,
        run: impl FnOnce(
            WatchOptions,
            Box<dyn WatchSession>,
        ) -> Result<(Watcher, Receiver<WatchEvent>), FrametimeError>,
    ) -> Result<(), String> {
        let mut slot = lock(&self.active);
        if slot
            .as_ref()
            .is_some_and(|a| !a.ended.load(Ordering::SeqCst))
        {
            return Err("уже йде стеження".into());
        }
        let inputs = Arc::new(Inputs::new());
        let queue = Arc::new(Mutex::new(Vec::new()));
        let session = RemoteWatchSession {
            inputs: inputs.clone(),
            queue: queue.clone(),
        };
        let (watcher, events) =
            run(spec.options(), Box::new(session)).map_err(|e| e.to_string())?;
        let watcher = Arc::new(Mutex::new(Some(watcher)));
        let ended = Arc::new(AtomicBool::new(false));
        let (q, w, e, i) = (
            queue.clone(),
            watcher.clone(),
            ended.clone(),
            inputs.clone(),
        );
        std::thread::Builder::new()
            .name("optimax-svc-watch".into())
            .spawn(move || loop {
                match events.recv_timeout(PUMP_TICK) {
                    Ok(ev) => {
                        let last = matches!(ev, WatchEvent::Ended(_));
                        lock(&q).push(ev.into());
                        if last {
                            e.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if i.age() > ABANDON {
                            tracing::info!("служба: інтерфейс зник — стеження зупинено");
                            if let Some(w) = lock(&w).take() {
                                w.stop();
                            }
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        e.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        *slot = Some(ActiveWatch {
            inputs,
            queue,
            watcher,
            ended,
        });
        Ok(())
    }

    /// Приймає активне вікно, клавішу й кнопку «зберегти» від інтерфейсу, віддає події.
    pub fn poll(
        &self,
        foreground_pid: Option<u32>,
        hotkey: bool,
        capture: bool,
    ) -> Result<Vec<WireWatchEvent>, String> {
        let mut slot = lock(&self.active);
        let Some(a) = slot.as_ref() else {
            return Err("стеження немає".into());
        };
        a.inputs.report(foreground_pid, hotkey);
        if capture {
            if let Some(w) = lock(&a.watcher).as_ref() {
                w.capture_now();
            }
        }
        let events = std::mem::take(&mut *lock(&a.queue));
        if a.ended.load(Ordering::SeqCst) && lock(&a.queue).is_empty() {
            *slot = None;
        }
        Ok(events)
    }

    /// Зупиняє стеження (подія `ended` прийде наступним опитуванням).
    pub fn stop(&self) {
        let watcher = lock(&self.active)
            .as_ref()
            .and_then(|a| lock(&a.watcher).take());
        if let Some(w) = watcher {
            w.stop();
        }
    }
}

struct ActiveLive {
    inputs: Arc<Inputs>,
    monitor: Arc<Mutex<Option<LiveMonitor>>>,
}

/// Живі дані оверлею в службі.
#[derive(Default)]
pub struct LiveHost {
    active: Mutex<Option<ActiveLive>>,
}

impl std::fmt::Debug for LiveHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveHost")
    }
}

impl LiveHost {
    /// Нічого не запускає.
    pub fn new() -> Self {
        Self::default()
    }

    /// Починає збір. Якщо вже йде — нічого не робить.
    pub fn start(
        &self,
        run: impl FnOnce(Foreground) -> Result<LiveMonitor, FrametimeError>,
    ) -> Result<(), String> {
        let mut slot = lock(&self.active);
        if slot.as_ref().is_some_and(|a| {
            lock(&a.monitor)
                .as_ref()
                .is_some_and(LiveMonitor::is_running)
        }) {
            return Ok(());
        }
        let inputs = Arc::new(Inputs::new());
        let fg = inputs.clone();
        let monitor = run(Arc::new(move || fg.foreground())).map_err(|e| e.to_string())?;
        let monitor = Arc::new(Mutex::new(Some(monitor)));
        let (m, i) = (monitor.clone(), inputs.clone());
        std::thread::Builder::new()
            .name("optimax-svc-live".into())
            .spawn(move || loop {
                std::thread::sleep(PUMP_TICK);
                let mut g = lock(&m);
                let Some(live) = g.as_ref() else { return };
                if i.age() > ABANDON || !live.is_running() {
                    if let Some(l) = g.take() {
                        drop(g);
                        l.stop();
                    }
                    return;
                }
            })
            .map_err(|e| e.to_string())?;
        *slot = Some(ActiveLive { inputs, monitor });
        Ok(())
    }

    /// Активне вікно від інтерфейсу; останні цифри. Помилка — збір зупинився.
    pub fn poll(&self, foreground_pid: Option<u32>) -> Result<Option<LiveStats>, String> {
        let slot = lock(&self.active);
        let Some(a) = slot.as_ref() else {
            return Err("оверлей не запущено".into());
        };
        a.inputs.report(foreground_pid, false);
        let monitor = lock(&a.monitor);
        let result = match monitor.as_ref() {
            Some(l) if l.is_running() => Ok(l.latest()),
            _ => Err("збір даних оверлею зупинився".into()),
        };
        result
    }

    /// Зупиняє збір.
    pub fn stop(&self) {
        let m = lock(&self.active)
            .take()
            .and_then(|a| lock(&a.monitor).take());
        if let Some(l) = m {
            l.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_frametime::watch::WatchStatus;

    #[test]
    fn spec_drops_session_only_parts() {
        let mut o = WatchOptions::new(Target::ProcessName("g.exe".into()));
        o.window = Duration::from_secs(20);
        o.freeze_ms = Some(100.0);
        o.sound = true;
        let s = WatchSpec::from_options(&o);
        let back = s.options();
        assert_eq!(back.window, Duration::from_secs(20));
        assert_eq!(back.freeze_ms, Some(100.0));
        assert!(!back.hotkey && !back.sound && back.notices.is_none());
    }

    #[test]
    fn events_roundtrip_through_the_pipe() {
        let status = WatchStatus {
            game_running: true,
            in_game: true,
            buffered_s: 12.0,
            dpc: true,
            played_s: 30.0,
        };
        for e in [
            WatchEvent::Status(status),
            WatchEvent::Pressed,
            WatchEvent::Ended(WatchEnd::GameExited),
        ] {
            let wire: WireWatchEvent = e.into();
            let json = serde_json::to_value(&wire).expect("json");
            let back: WireWatchEvent = serde_json::from_value(json).expect("назад");
            assert!(back.into_event().is_ok());
        }
        let n = WireWatchEvent::Notice {
            notice: WatchNotice::Saving,
        };
        let back: WireWatchEvent =
            serde_json::from_value(serde_json::to_value(&n).expect("json")).expect("назад");
        assert_eq!(back.into_event().err(), Some(WatchNotice::Saving));
    }
}
