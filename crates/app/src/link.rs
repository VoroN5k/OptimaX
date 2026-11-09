//! Куди йти по дані, що потребують прав адміністратора: у службу OptimaX (якщо вона працює й
//! прийме цей exe) чи в сам процес програми. Служба відмовила — тихо читаємо самі.

use crate::error::{CmdError, CmdResult};
use crate::state::AppState;
use optimax_backend::record::{PollReply, RecordSpec};
use optimax_backend::sensors::{SensorCache, SensorRead};
use optimax_backend::watch::{WatchSpec, WireWatchEvent};
use optimax_frametime::live::{LiveMonitor, LiveStats};
use optimax_frametime::watch::{
    LocalWatchSession, WatchEnd, WatchEvent, WatchOptions, WatchSession, Watcher,
};
use optimax_frametime::{
    record_with, CancelToken, LocalSession, RecordOptions, RecordProgress, Recording, UserSession,
};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use optimax_service::client::Client;
use optimax_service::install::{self, SvcState};
use optimax_service::protocol::Command;
use optimax_service::{same_file, service_exe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Як часто перепитувати диспетчер служб, чи служба доступна.
const RECHECK: Duration = Duration::from_secs(10);
/// Як часто опитувати службу під час запису (активне вікно, клавіша, хід).
const POLL: Duration = Duration::from_millis(100);
/// Як часто передавати хід запису в інтерфейс.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Вибір шляху й локальний кеш датчиків.
#[derive(Debug, Default)]
pub struct Link {
    local: SensorCache,
    usable: Mutex<Option<(Instant, bool)>>,
}

impl Link {
    /// Нічого не відкриває, доки не попросять.
    pub fn new() -> Self {
        Self::default()
    }

    /// Служба працює, і ця програма запущена з її exe (інакше служба з'єднання не прийме).
    pub fn service_usable(&self) -> bool {
        let mut slot = self.usable.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, v)) = *slot {
            if at.elapsed() < RECHECK {
                return v;
            }
        }
        let v = install::state().is_ok_and(|s| s == SvcState::Running)
            && std::env::current_exe()
                .ok()
                .zip(service_exe())
                .is_some_and(|(me, svc)| same_file(&me, &svc));
        *slot = Some((Instant::now(), v));
        v
    }

    fn forget_service(&self, why: &dyn std::fmt::Display) {
        tracing::warn!("служба OptimaX не відповіла, читаю сам: {why}");
        *self.usable.lock().unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), false));
    }

    fn via_service<T: serde::de::DeserializeOwned>(&self, cmd: Command) -> Option<T> {
        if !self.service_usable() {
            return None;
        }
        match Client::connect().and_then(|mut c| c.json::<T>(cmd)) {
            Ok(v) => Some(v),
            Err(e) => {
                self.forget_service(&e);
                None
            }
        }
    }

    /// Датчики PawnIO і чи прочитано через службу.
    pub fn sensors(&self) -> (SensorRead, bool) {
        match self.via_service(Command::Sensors) {
            Some(r) => (r, true),
            None => (self.local.read(), false),
        }
    }

    /// Лише температура CPU.
    pub fn cpu_temp(&self) -> Option<f32> {
        match self.via_service::<Option<f32>>(Command::CpuTemp) {
            Some(t) => t,
            None => self.local.temp(),
        }
    }

    /// Запис гри: через службу, якщо вона доступна (PresentMon і ETW — там, активне вікно,
    /// клавіша, звуки й повідомлення — тут), інакше в цьому процесі. Служба не прийняла запис —
    /// пишемо самі; служба зникла посеред запису — помилка (стан запису невідомий).
    pub fn record(
        &self,
        state: &AppState,
        options: &RecordOptions,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(&RecordProgress),
    ) -> CmdResult<Recording> {
        if self.service_usable() {
            // Клавіша й повідомлення — у сесії користувача; реєструємо до старту служби.
            let session = LocalSession::new(options)?;
            match start_remote(options) {
                Ok(client) => return remote_loop(client, &session, cancel, on_progress),
                Err(e) => self.forget_service(&e),
            }
        }
        let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
        Ok(record_with(
            state.presentmon()?,
            options,
            Box::new(SysinfoSource::new()),
            gpu,
            cancel,
            on_progress,
        )?)
    }
}

fn service_err(e: optimax_service::ServiceError) -> CmdError {
    CmdError::Failed(format!("служба OptimaX: {e}"))
}

/// Під'єднується й просить службу почати запис.
fn start_remote(options: &RecordOptions) -> Result<Client, optimax_service::ServiceError> {
    let mut c = Client::connect()?;
    let spec = serde_json::to_value(RecordSpec::from_options(options)).map_err(|e| {
        optimax_service::ServiceError::Protocol(optimax_service::protocol::ProtocolError::Json(e))
    })?;
    c.json::<serde_json::Value>(Command::RecordStart { spec })?;
    tracing::info!("запис гри — через службу OptimaX");
    Ok(c)
}

/// Опитує службу до кінця запису: передає активне вікно й клавішу, відтворює звуки й повідомлення,
/// віддає хід.
fn remote_loop(
    mut c: Client,
    session: &LocalSession,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(&RecordProgress),
) -> CmdResult<Recording> {
    let mut cancel_sent = false;
    let mut last: Option<(Instant, optimax_frametime::Phase)> = None;
    loop {
        std::thread::sleep(POLL);
        if cancel.is_cancelled() && !cancel_sent {
            cancel_sent = true;
            c.json::<serde_json::Value>(Command::RecordCancel)
                .map_err(service_err)?;
        }
        let reply: PollReply = c
            .json(Command::RecordPoll {
                foreground_pid: session.foreground_pid(),
                hotkey: session.hotkey_pressed(),
            })
            .map_err(service_err)?;
        for e in reply.events {
            session.event(e);
        }
        if let Some(p) = &reply.progress {
            let due =
                last.is_none_or(|(at, phase)| phase != p.phase || at.elapsed() >= PROGRESS_EVERY);
            if due {
                on_progress(p);
                last = Some((Instant::now(), p.phase));
            }
        }
        if let Some(done) = reply.done {
            return done.map_err(CmdError::from);
        }
    }
}

// ---- Стеження («Щойно лагнуло?», чорна скринька) і оверлей ----

/// Стеження: у цьому процесі або через службу. Той самий набір дій, що й [`Watcher`].
pub enum WatchHandle {
    /// У цьому процесі.
    Local(Watcher),
    /// Через службу: потік опитування.
    Remote(RemoteWatch),
}

impl std::fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Local(_) => "WatchHandle::Local",
            Self::Remote(_) => "WatchHandle::Remote",
        })
    }
}

impl WatchHandle {
    /// Те саме, що гаряча клавіша: зберегти останні секунди.
    pub fn capture_now(&self) {
        match self {
            Self::Local(w) => w.capture_now(),
            Self::Remote(r) => r.capture.store(true, Ordering::SeqCst),
        }
    }

    /// Зупиняє й чекає завершення.
    pub fn stop(self) {
        match self {
            Self::Local(w) => w.stop(),
            Self::Remote(mut r) => r.shutdown(),
        }
    }
}

/// Стеження через службу.
pub struct RemoteWatch {
    stop: Arc<AtomicBool>,
    capture: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl RemoteWatch {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for RemoteWatch {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Link {
    /// Починає стеження: через службу, якщо вона доступна, інакше в цьому процесі. Події — у
    /// каналі, як у [`Watcher::start`].
    pub fn start_watch(
        &self,
        state: &AppState,
        options: WatchOptions,
    ) -> CmdResult<(WatchHandle, Receiver<WatchEvent>)> {
        if self.service_usable() {
            // Клавіша, звук і повідомлення — тут, у сесії користувача.
            let session = LocalWatchSession::new(&options)?;
            match start_remote_watch(&options) {
                Ok(client) => {
                    let (handle, rx) = remote_watch(client, session)?;
                    return Ok((WatchHandle::Remote(handle), rx));
                }
                Err(e) => self.forget_service(&e),
            }
        }
        let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
        let (w, rx) = Watcher::start(
            state.presentmon()?,
            options,
            Box::new(SysinfoSource::new()),
            gpu,
        )?;
        Ok((WatchHandle::Local(w), rx))
    }

    /// Живі дані оверлею: через службу, якщо вона доступна, інакше в цьому процесі.
    pub fn start_live(&self, state: &AppState) -> CmdResult<LiveHandle> {
        if self.service_usable() {
            match Client::connect().and_then(|mut c| {
                c.json::<serde_json::Value>(Command::LiveStart)?;
                Ok(c)
            }) {
                Ok(c) => {
                    tracing::info!("оверлей — дані через службу OptimaX");
                    return Ok(LiveHandle::Remote(Mutex::new(RemoteLive {
                        client: c,
                        ok: true,
                    })));
                }
                Err(e) => self.forget_service(&e),
            }
        }
        let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
        Ok(LiveHandle::Local(LiveMonitor::start(
            state.presentmon()?,
            Box::new(SysinfoSource::new()),
            gpu,
        )?))
    }
}

fn start_remote_watch(options: &WatchOptions) -> Result<Client, optimax_service::ServiceError> {
    let mut c = Client::connect()?;
    let spec = serde_json::to_value(WatchSpec::from_options(options)).map_err(|e| {
        optimax_service::ServiceError::Protocol(optimax_service::protocol::ProtocolError::Json(e))
    })?;
    c.json::<serde_json::Value>(Command::WatchStart { spec })?;
    tracing::info!("стеження — через службу OptimaX");
    Ok(c)
}

/// Потік опитування стеження в службі: активне вікно, клавіша, кнопка «зберегти» — туди; події,
/// повідомлення — звідти.
fn remote_watch(
    mut c: Client,
    session: LocalWatchSession,
) -> CmdResult<(RemoteWatch, Receiver<WatchEvent>)> {
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let capture = Arc::new(AtomicBool::new(false));
    let (s, cap) = (stop.clone(), capture.clone());
    let handle = std::thread::Builder::new()
        .name("optimax-watch-remote".into())
        .spawn(move || {
            let mut stopping = false;
            loop {
                std::thread::sleep(POLL);
                if s.load(Ordering::SeqCst) && !stopping {
                    stopping = true;
                    let _ = c.json::<serde_json::Value>(Command::WatchStop);
                }
                let events: Vec<WireWatchEvent> = match c.json(Command::WatchPoll {
                    foreground_pid: session.foreground_pid(),
                    hotkey: session.hotkey_pressed(),
                    capture: cap.swap(false, Ordering::SeqCst),
                }) {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = tx.send(WatchEvent::Ended(WatchEnd::PresentMonFailed(format!(
                            "служба OptimaX: {e}"
                        ))));
                        return;
                    }
                };
                for e in events {
                    match e.into_event() {
                        Ok(ev) => {
                            let last = matches!(ev, WatchEvent::Ended(_));
                            if tx.send(ev).is_err() || last {
                                return;
                            }
                        }
                        Err(notice) => session.notice(notice),
                    }
                }
            }
        })
        .map_err(|e| CmdError::Failed(e.to_string()))?;
    Ok((
        RemoteWatch {
            stop,
            capture,
            handle: Some(handle),
        },
        rx,
    ))
}

/// Оверлей через службу: опитування в тому самому темпі, що й оновлення оверлею.
pub struct RemoteLive {
    client: Client,
    ok: bool,
}

/// Живі дані оверлею: у цьому процесі або через службу.
pub enum LiveHandle {
    /// У цьому процесі.
    Local(LiveMonitor),
    /// Через службу.
    Remote(Mutex<RemoteLive>),
}

impl std::fmt::Debug for LiveHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveHandle")
    }
}

impl LiveHandle {
    /// Останні цифри (через службу — і передає їй активне вікно).
    pub fn latest(&self) -> Option<LiveStats> {
        match self {
            Self::Local(l) => l.latest(),
            Self::Remote(r) => {
                let mut r = r.lock().unwrap_or_else(PoisonError::into_inner);
                let own = std::process::id();
                let fg = optimax_core::input::foreground_process_id().filter(|p| *p != own);
                match r
                    .client
                    .json::<Option<LiveStats>>(Command::LivePoll { foreground_pid: fg })
                {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("оверлей: служба не відповіла: {e}");
                        r.ok = false;
                        None
                    }
                }
            }
        }
    }

    /// Чи збір ще йде.
    pub fn is_running(&self) -> bool {
        match self {
            Self::Local(l) => l.is_running(),
            Self::Remote(r) => r.lock().unwrap_or_else(PoisonError::into_inner).ok,
        }
    }

    /// Зупиняє збір.
    pub fn stop(self) {
        match self {
            Self::Local(l) => l.stop(),
            Self::Remote(r) => {
                let mut r = r.into_inner().unwrap_or_else(PoisonError::into_inner);
                let _ = r.client.json::<serde_json::Value>(Command::LiveStop);
            }
        }
    }
}
