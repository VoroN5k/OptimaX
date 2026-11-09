//! Запис гри у службі (етап 2б): PresentMon, ETW і метрики — тут, із правами SYSTEM; активне
//! вікно, гаряча клавіша, звуки й повідомлення — в інтерфейсі, який раз на ~100 мс опитує службу
//! командою `record_poll` і передає їх.
//!
//! Один запис одночасно. Якщо інтерфейс зник (не опитує понад [`ABANDON`]), запис скасовується.

use optimax_frametime::{
    gate::EndReason, CancelToken, FrametimeError, RecordOptions, RecordProgress, Recording,
    SessionEvent, StartMode, Target, UserSession,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Без опитування довше за це гра вважається не в фокусі (лічба стоїть).
pub const STALE: Duration = Duration::from_secs(2);
/// Без опитування довше за це запис скасовується: інтерфейс, імовірно, закрито.
pub const ABANDON: Duration = Duration::from_secs(30);

/// Що записувати (передається каналом; звуки й повідомлення лишаються в інтерфейсі).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordSpec {
    /// Ціль.
    pub target: Target,
    /// Скільки чистої гри зарахувати, мс.
    pub duration_ms: u64,
    /// Як починати.
    pub start: StartMode,
    /// Скільки чекати старту, мс.
    pub max_wait_ms: u64,
    /// Читати температуру CPU (користувач дав згоду на PawnIO).
    pub cpu_temp: bool,
}

impl RecordSpec {
    /// З параметрів запису інтерфейсу.
    pub fn from_options(o: &RecordOptions) -> Self {
        Self {
            target: o.target.clone(),
            duration_ms: u64::try_from(o.duration.as_millis()).unwrap_or(u64::MAX),
            start: o.start,
            max_wait_ms: u64::try_from(o.max_wait.as_millis()).unwrap_or(u64::MAX),
            cpu_temp: o.sampler.cpu_temp.is_some(),
        }
    }

    /// Параметри запису в службі: без звуків і повідомлень (їх відтворює інтерфейс).
    pub fn options(&self) -> RecordOptions {
        let mut o =
            RecordOptions::new(self.target.clone(), Duration::from_millis(self.duration_ms))
                .with_start(self.start, false);
        o.max_wait = Duration::from_millis(self.max_wait_ms);
        o
    }
}

/// Чому запис не вдався (зберігає те, що інтерфейс показує по-своєму).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordFailure {
    /// PresentMon не записав кадрів.
    NoFrames(String),
    /// Запис не почався.
    NotStarted(EndReason),
    /// Інше.
    Other(String),
}

impl From<FrametimeError> for RecordFailure {
    fn from(e: FrametimeError) -> Self {
        match e {
            FrametimeError::NoFrames(d) => Self::NoFrames(d),
            FrametimeError::NotStarted(r) => Self::NotStarted(r),
            other => Self::Other(other.to_string()),
        }
    }
}

/// Відповідь на опитування.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollReply {
    /// Останній хід запису.
    pub progress: Option<RecordProgress>,
    /// Події від минулого опитування (звуки, повідомлення).
    pub events: Vec<SessionEvent>,
    /// Запис завершився (віддається один раз).
    pub done: Option<Result<Recording, RecordFailure>>,
}

/// Що повідомив інтерфейс.
#[derive(Debug)]
struct Inputs {
    foreground: Mutex<(Option<u32>, Instant)>,
    hotkey: AtomicBool,
}

#[derive(Debug, Default)]
struct Shared {
    progress: Option<RecordProgress>,
    events: Vec<SessionEvent>,
    done: Option<Result<Recording, RecordFailure>>,
}

#[derive(Debug)]
struct Active {
    inputs: Arc<Inputs>,
    cancel: CancelToken,
    shared: Arc<Mutex<Shared>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Сесія користувача, яку передає інтерфейс через канал.
struct RemoteSession {
    inputs: Arc<Inputs>,
    shared: Arc<Mutex<Shared>>,
    cancel: CancelToken,
}

impl UserSession for RemoteSession {
    fn foreground_pid(&self) -> Option<u32> {
        let (pid, at) = *lock(&self.inputs.foreground);
        let age = at.elapsed();
        if age > ABANDON {
            self.cancel.cancel();
        }
        pid.filter(|_| age <= STALE)
    }

    fn hotkey_pressed(&self) -> bool {
        self.inputs.hotkey.swap(false, Ordering::SeqCst)
    }

    fn event(&self, e: SessionEvent) {
        lock(&self.shared).events.push(e);
    }
}

/// Запис у службі (один одночасно).
#[derive(Debug, Default)]
pub struct Recorder {
    active: Mutex<Option<Active>>,
}

impl Recorder {
    /// Нічого не запускає.
    pub fn new() -> Self {
        Self::default()
    }

    /// Починає запис у власному потоці. `run` — сам запис (у службі — PresentMon і метрики).
    pub fn start(
        &self,
        spec: RecordSpec,
        run: impl FnOnce(
                &RecordOptions,
                &CancelToken,
                &dyn UserSession,
                &mut dyn FnMut(&RecordProgress),
            ) -> Result<Recording, FrametimeError>
            + Send
            + 'static,
    ) -> Result<(), String> {
        let mut slot = lock(&self.active);
        if slot
            .as_ref()
            .is_some_and(|a| lock(&a.shared).done.is_none())
        {
            return Err("уже йде запис".into());
        }
        let inputs = Arc::new(Inputs {
            foreground: Mutex::new((None, Instant::now())),
            hotkey: AtomicBool::new(false),
        });
        let shared = Arc::new(Mutex::new(Shared::default()));
        let cancel = CancelToken::new();
        let session = RemoteSession {
            inputs: inputs.clone(),
            shared: shared.clone(),
            cancel: cancel.clone(),
        };
        let (thread_shared, thread_cancel) = (shared.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("optimax-svc-record".into())
            .spawn(move || {
                let options = spec.options();
                let progress_shared = thread_shared.clone();
                let mut on_progress = move |p: &RecordProgress| {
                    lock(&progress_shared).progress = Some(p.clone());
                };
                let result = run(&options, &thread_cancel, &session, &mut on_progress)
                    .map_err(RecordFailure::from);
                lock(&thread_shared).done = Some(result);
            })
            .map_err(|e| e.to_string())?;
        *slot = Some(Active {
            inputs,
            cancel,
            shared,
        });
        Ok(())
    }

    /// Приймає активне вікно й клавішу від інтерфейсу, віддає хід, події й (раз) результат.
    pub fn poll(&self, foreground_pid: Option<u32>, hotkey: bool) -> Result<PollReply, String> {
        let mut slot = lock(&self.active);
        let Some(a) = slot.as_ref() else {
            return Err("запису немає".into());
        };
        *lock(&a.inputs.foreground) = (foreground_pid, Instant::now());
        if hotkey {
            a.inputs.hotkey.store(true, Ordering::SeqCst);
        }
        let mut s = lock(&a.shared);
        let reply = PollReply {
            progress: s.progress.clone(),
            events: std::mem::take(&mut s.events),
            done: s.done.take(),
        };
        drop(s);
        if reply.done.is_some() {
            *slot = None;
        }
        Ok(reply)
    }

    /// Просить зупинити запис.
    pub fn cancel(&self) {
        if let Some(a) = lock(&self.active).as_ref() {
            a.cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_frametime::Phase;

    fn spec() -> RecordSpec {
        RecordSpec {
            target: Target::ProcessName("game.exe".into()),
            duration_ms: 30_000,
            start: StartMode::Auto,
            max_wait_ms: 600_000,
            cpu_temp: true,
        }
    }

    fn wait_done(r: &Recorder) -> PollReply {
        for _ in 0..200 {
            let p = r.poll(Some(42), false).expect("poll");
            if p.done.is_some() {
                return p;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("запис не завершився");
    }

    #[test]
    fn spec_roundtrips_options_without_sound_or_notices() {
        let s = spec();
        let o = s.options();
        assert_eq!(o.duration, Duration::from_secs(30));
        assert_eq!(o.start, StartMode::Auto);
        assert!(!o.sound);
        assert!(o.notices.is_none());
        let back: RecordSpec =
            serde_json::from_value(serde_json::to_value(&s).expect("json")).expect("назад");
        assert_eq!(back, s);
    }

    #[test]
    fn session_inputs_events_and_single_result() {
        let r = Recorder::new();
        r.start(spec(), |_, _, session, on_progress| {
            // Чекаємо, поки інтерфейс передасть активне вікно.
            for _ in 0..200 {
                if session.foreground_pid() == Some(42) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(session.foreground_pid(), Some(42));
            session.event(SessionEvent::Started);
            on_progress(&RecordProgress {
                phase: Phase::Recording,
                counted_s: 1.0,
                target_s: 30.0,
                elapsed_s: 1.0,
            });
            Err(FrametimeError::NoFrames("тест".into()))
        })
        .expect("старт");
        // Другий запис, поки йде перший, — відмова.
        assert!(r
            .start(spec(), |_, _, _, _| Err(FrametimeError::NoFrames(
                String::new()
            )))
            .is_err());
        let p = wait_done(&r);
        assert_eq!(
            p.done.map(|d| d.err()),
            Some(Some(RecordFailure::NoFrames("тест".into())))
        );
        // Результат віддається один раз, далі запису немає.
        assert!(r.poll(None, false).is_err());
    }

    #[test]
    fn stale_interface_means_not_focused() {
        let inputs = Arc::new(Inputs {
            foreground: Mutex::new((Some(7), Instant::now() - STALE - Duration::from_millis(1))),
            hotkey: AtomicBool::new(true),
        });
        let s = RemoteSession {
            inputs,
            shared: Arc::new(Mutex::new(Shared::default())),
            cancel: CancelToken::new(),
        };
        assert_eq!(s.foreground_pid(), None);
        // Натискання клавіші читається один раз.
        assert!(s.hotkey_pressed());
        assert!(!s.hotkey_pressed());
    }
}
