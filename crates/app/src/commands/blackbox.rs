//! Чорна скринька: поки йде сесія режиму гри для профілю з `black_box`, працює стеження
//! «Щойно лагнуло» з автоматичними знімками фризів (кадр ≥ [`FREEZE_MS`]). Для кожного фризу —
//! можливі причини (збіг у часі з ознаками, `optimax_diagnosis::freeze_causes`); наприкінці гри
//! підсумок сесії й знімки найгірших фризів зберігаються в історію (`history/sessions`).
//!
//! Запис «Діагностики», A/B, тест роздільності й «Щойно лагнуло» ставлять скриньку на паузу
//! ([`Pause`]): у системі одна сесія PresentMon і одна DPC. Після паузи стеження само
//! відновлюється через [`RESUME_DELAY`]. Нічого в системі не змінює.
//! Подія `blackbox` ([`BlackBoxView`]) — при зміні стану.

use super::diagnosis::shader_cache_files;
use super::history::{compress, system_state};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_audit::{SystemProbe, WindowsProbe};
use optimax_diagnosis::{freeze_causes, Context};
use optimax_frametime::watch::{FreezeMark, WatchEnd, WatchEvent, WatchOptions, Watcher};
use optimax_frametime::{Recording, Target};
use optimax_history::{
    cause_summary, entry_id, keep_worst, FreezeCauseCount, FreezeEntry, SessionEntry, SessionStore,
    StdDir, KEEP_FREEZE_RECORDINGS, KEEP_SESSIONS,
};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Кадр від стількох мс — фриз (погоджено користувачем 2026-11-03).
pub const FREEZE_MS: f64 = 100.0;
/// Через скільки після паузи стеження відновлюється (A/B робить кілька записів поспіль).
const RESUME_DELAY: Duration = Duration::from_secs(10);
/// Пауза між спробами запустити стеження, якщо воно не стартувало.
const RETRY_DELAY: Duration = Duration::from_secs(30);
/// Такт наглядача сесії.
const SUPERVISE_EVERY: Duration = Duration::from_secs(1);

/// Поточна сесія для інтерфейсу.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActiveSession {
    /// Гра.
    pub game: String,
    /// Початок, Unix мс.
    pub started_unix_ms: i64,
    /// Секунд зарахованої гри.
    pub played_s: f64,
    /// Фризів поки що.
    pub freezes: u32,
    /// Стеження на паузі (іде інший запис).
    pub paused: bool,
}

/// Остання збережена сесія зі зведенням причин.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SessionSummary {
    /// Сесія.
    pub session: SessionEntry,
    /// Фризи за головною причиною.
    pub causes: Vec<FreezeCauseCount>,
}

/// Стан чорної скриньки.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BlackBoxView {
    /// Сесія, що йде зараз.
    pub active: Option<ActiveSession>,
    /// Найновіша збережена сесія.
    pub last: Option<SessionSummary>,
}

/// Сесія, що йде.
pub struct BlackBox {
    /// Номер (події старої сесії ігноруються).
    id: u64,
    session: SessionEntry,
    display_hz: Option<u32>,
    watcher: Option<Watcher>,
    pump: Option<JoinHandle<()>>,
    /// Номер запуску стеження в межах сесії (події попереднього запуску ігноруються).
    generation: u64,
    played_before_s: f64,
    played_now_s: f64,
    pauses: u32,
    paused_since: Option<Instant>,
    resume_after: Option<Instant>,
    next_recording: u32,
    stop: Arc<AtomicBool>,
    supervisor: Option<JoinHandle<()>>,
}

impl BlackBox {
    fn active(&self) -> ActiveSession {
        ActiveSession {
            game: self.session.game.clone(),
            started_unix_ms: self.session.started_unix_ms,
            played_s: self.played_before_s + self.played_now_s,
            freezes: u32::try_from(self.session.freezes.len()).unwrap_or(u32::MAX),
            paused: self.watcher.is_none(),
        }
    }
}

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

fn sessions_dir() -> CmdResult<StdDir> {
    let base = optimax_core::datadir::data_dir().map_err(failed)?;
    StdDir::new(base.join("history").join("sessions")).map_err(failed)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Іде інший запис гри (тоді стеження не запускаємо).
fn busy(state: &AppState) -> bool {
    lock(&state.recording).is_some()
        || lock(&state.ab).is_some()
        || super::resolution::is_running(state)
        || super::lag::is_watching(state)
}

fn view(state: &AppState) -> BlackBoxView {
    let active = lock(&state.blackbox).as_ref().map(BlackBox::active);
    BlackBoxView {
        active,
        last: last_summary(),
    }
}

fn last_summary() -> Option<SessionSummary> {
    let files = sessions_dir().ok()?;
    let (sessions, _) = SessionStore::new(&files).list().ok()?;
    let session = sessions.into_iter().next()?;
    Some(SessionSummary {
        causes: cause_summary(&session),
        session,
    })
}

fn emit(app: &AppHandle) {
    let _ = app.emit("blackbox", view(&app.state::<AppState>()));
}

/// Починає сесію для гри (з потоку режиму гри). Попередня сесія, якщо є, завершується.
pub fn start(app: &AppHandle, game: &str) {
    finish(app);
    let state = app.state::<AppState>();
    let started = now_ms();
    let display_hz = WindowsProbe::new()
        .displays()
        .ok()
        .and_then(|d| d.into_iter().find(|d| d.primary).map(|d| d.current_hz));
    let stop = Arc::new(AtomicBool::new(false));
    let id = {
        let mut slot = lock(&state.blackbox_seq);
        *slot += 1;
        *slot
    };
    let session = SessionEntry {
        id: entry_id(game, started),
        game: game.to_string(),
        started_unix_ms: started,
        ended_unix_ms: started,
        played_s: 0.0,
        paused_s: 0.0,
        threshold_ms: FREEZE_MS,
        freezes: Vec::new(),
        state: system_state(game),
    };
    let supervisor = {
        let app = app.clone();
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("optimax-blackbox".into())
            .spawn(move || supervise(&app, id, &stop))
    };
    let supervisor = match supervisor {
        Ok(h) => Some(h),
        Err(e) => {
            tracing::error!("чорна скринька не почалась: {e}");
            return;
        }
    };
    *lock(&state.blackbox) = Some(BlackBox {
        id,
        session,
        display_hz,
        watcher: None,
        pump: None,
        generation: 0,
        played_before_s: 0.0,
        played_now_s: 0.0,
        pauses: 0,
        paused_since: None,
        resume_after: None,
        next_recording: 1,
        stop,
        supervisor,
    });
    tracing::info!("чорна скринька: сесія {game} почалась (фриз — кадр від {FREEZE_MS} мс)");
    emit(app);
}

/// Наглядач: запускає стеження, коли нічого не заважає, і перезапускає після паузи.
fn supervise(app: &AppHandle, id: u64, stop: &AtomicBool) {
    let state = app.state::<AppState>();
    let mut next_try = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(SUPERVISE_EVERY);
        if stop.load(Ordering::SeqCst) || Instant::now() < next_try || busy(&state) {
            continue;
        }
        let game = {
            let slot = lock(&state.blackbox);
            match slot.as_ref().filter(|b| b.id == id) {
                Some(b)
                    if b.watcher.is_none()
                        && b.pauses == 0
                        && b.resume_after.is_none_or(|t| Instant::now() >= t) =>
                {
                    b.session.game.clone()
                }
                Some(_) => continue,
                None => return,
            }
        };
        match start_watcher(app, &state, id, &game) {
            Ok(()) => emit(app),
            Err(e) => {
                tracing::warn!("чорна скринька: стеження не запустилось: {}", e.message());
                next_try = Instant::now() + RETRY_DELAY;
            }
        }
    }
}

fn start_watcher(app: &AppHandle, state: &AppState, id: u64, game: &str) -> CmdResult<()> {
    let mut options = WatchOptions::new(Target::ProcessName(game.to_string()));
    options.hotkey = false;
    options.freeze_ms = Some(FREEZE_MS);
    let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
    let (watcher, events) = Watcher::start(
        state.presentmon()?,
        options,
        Box::new(SysinfoSource::new()),
        gpu,
    )?;
    let mut slot = lock(&state.blackbox);
    let Some(b) = slot.as_mut().filter(|b| b.id == id && b.watcher.is_none()) else {
        // Сесію закрили, поки стеження запускалось.
        drop(slot);
        watcher.stop();
        return Ok(());
    };
    b.generation += 1;
    let generation = b.generation;
    if let Some(since) = b.paused_since.take() {
        b.session.paused_s += since.elapsed().as_secs_f64();
    }
    b.watcher = Some(watcher);
    let app2 = app.clone();
    b.pump = std::thread::Builder::new()
        .name("optimax-blackbox-events".into())
        .spawn(move || pump(&app2, id, generation, &events))
        .ok();
    Ok(())
}

/// Події одного запуску стеження.
fn pump(app: &AppHandle, id: u64, generation: u64, events: &Receiver<WatchEvent>) {
    let state = app.state::<AppState>();
    let same = |b: &BlackBox| b.id == id && b.generation == generation;
    for event in events {
        match event {
            WatchEvent::Status(s) => {
                if let Some(b) = lock(&state.blackbox).as_mut().filter(|b| same(b)) {
                    b.played_now_s = s.played_s;
                }
            }
            WatchEvent::Freezes {
                recording,
                freezes,
                failure,
            } => {
                if let Some(f) = failure {
                    tracing::info!("чорна скринька: знімок фризу не вдався ({f:?})");
                }
                record_freezes(app, id, recording.as_deref(), &freezes);
                emit(app);
            }
            WatchEvent::Ended(end) => {
                if let WatchEnd::PresentMonFailed(m) = &end {
                    tracing::error!("чорна скринька: PresentMon не працює: {m}");
                }
                let watcher = {
                    let mut slot = lock(&state.blackbox);
                    match slot.as_mut().filter(|b| same(b)) {
                        Some(b) => {
                            b.played_before_s += b.played_now_s;
                            b.played_now_s = 0.0;
                            if b.paused_since.is_none() {
                                b.paused_since = Some(Instant::now());
                            }
                            if matches!(end, WatchEnd::PresentMonFailed(_)) {
                                b.resume_after = Some(Instant::now() + RETRY_DELAY);
                            }
                            b.watcher.take()
                        }
                        None => None,
                    }
                };
                // Потік стеження вже завершився — звільняємо без очікування.
                drop(watcher);
                emit(app);
                return;
            }
            WatchEvent::Pressed
            | WatchEvent::Captured { .. }
            | WatchEvent::CaptureFailed { .. } => {}
        }
    }
}

/// Аналізує фризи знімка і додає їх у сесію; знімок — на диск.
fn record_freezes(app: &AppHandle, id: u64, rec: Option<&Recording>, marks: &[FreezeMark]) {
    let state = app.state::<AppState>();
    let Some(display_hz) = lock(&state.blackbox)
        .as_ref()
        .filter(|b| b.id == id)
        .map(|b| b.display_hz)
    else {
        return;
    };
    let causes = match rec {
        Some(r) => {
            let ctx = Context {
                display_hz,
                config_findings: Vec::new(),
                shader_cache_files: r.start_unix_ms.map(shader_cache_files).unwrap_or_default(),
            };
            let times: Vec<f64> = marks.iter().map(|m| m.t_ms).collect();
            freeze_causes(r, &ctx, &times)
        }
        None => vec![Vec::new(); marks.len()],
    };
    let bytes = rec.and_then(|r| compress(r).ok());
    let mut slot = lock(&state.blackbox);
    let Some(b) = slot.as_mut().filter(|b| b.id == id) else {
        return;
    };
    let number = bytes.as_ref().map(|_| {
        let n = b.next_recording;
        b.next_recording += 1;
        n
    });
    for (m, c) in marks.iter().zip(causes) {
        b.session.freezes.push(FreezeEntry {
            at_unix_ms: m.at_unix_ms,
            frame_ms: m.frame_ms,
            causes: c.iter().map(|k| k.code().to_string()).collect(),
            recording: number,
            t_ms: m.t_ms,
        });
    }
    b.session.ended_unix_ms = now_ms();
    b.session.played_s = b.played_before_s + b.played_now_s;
    let snapshot = b.session.clone();
    drop(slot);
    // Підсумок на диску вже під час гри: якщо програма впаде, сесія не загубиться.
    let saved = sessions_dir().and_then(|files| {
        let store = SessionStore::new(&files);
        if let (Some(n), Some(bytes)) = (number, &bytes) {
            store
                .save_recording(&snapshot.id, n, bytes)
                .map_err(failed)?;
        }
        store.save(&snapshot).map_err(failed)
    });
    if let Err(e) = saved {
        tracing::warn!("чорна скринька: не вдалося зберегти: {}", e.message());
    }
}

/// Завершує сесію: зупиняє стеження, зберігає підсумок і знімки найгірших фризів.
pub fn finish(app: &AppHandle) {
    if let Some(s) = finish_state(&app.state::<AppState>()) {
        let summary = cause_summary(&s)
            .iter()
            .map(|c| format!("{} {}", c.cause, c.count))
            .collect::<Vec<_>>()
            .join(", ");
        tracing::info!(
            "чорна скринька: {} — {:.0} хв гри, фризів {} ({summary})",
            s.game,
            s.played_s / 60.0,
            s.freezes.len()
        );
        emit(app);
    }
}

/// Те саме без подій інтерфейсу (вихід із програми). Повертає збережену сесію.
pub fn finish_state(state: &AppState) -> Option<SessionEntry> {
    // Спершу наглядач (щоб не запустив стеження заново), потім стеження: його зупинка
    // дописує фризи, що ще чекали знімка, і потік подій кладе їх у сесію — тому сесія
    // лишається в стані, доки потік подій не завершиться.
    let supervisor = {
        let mut slot = lock(&state.blackbox);
        let b = slot.as_mut()?;
        b.stop.store(true, Ordering::SeqCst);
        b.supervisor.take()
    };
    if let Some(s) = supervisor {
        let _ = s.join();
    }
    let (watcher, pump) = {
        let mut slot = lock(&state.blackbox);
        let b = slot.as_mut()?;
        (b.watcher.take(), b.pump.take())
    };
    if let Some(w) = watcher {
        w.stop();
    }
    if let Some(p) = pump {
        let _ = p.join();
    }
    let b = lock(&state.blackbox).take()?;
    let mut session = b.session;
    session.ended_unix_ms = now_ms();
    session.played_s = b.played_before_s + b.played_now_s;
    if let Some(since) = b.paused_since {
        session.paused_s += since.elapsed().as_secs_f64();
    }
    let removed = keep_worst(&mut session, KEEP_FREEZE_RECORDINGS);
    let saved = sessions_dir().and_then(|files| {
        let store = SessionStore::new(&files);
        for n in removed {
            store.remove_recording(&session.id, n).map_err(failed)?;
        }
        store.save(&session).map_err(failed)?;
        store.prune(KEEP_SESSIONS).map_err(failed)
    });
    if let Err(e) = saved {
        tracing::warn!("чорна скринька: сесію не збережено: {}", e.message());
    }
    Some(session)
}

/// Пауза скриньки на час іншого запису гри. Знімається при знищенні; стеження відновлюється
/// через [`RESUME_DELAY`], якщо нових пауз не було.
pub struct Pause {
    app: AppHandle,
    id: Option<u64>,
}

impl Pause {
    /// Ставить на паузу (блокує до зупинки стеження, ≈1 с). Без сесії — нічого не робить.
    pub fn new(app: &AppHandle) -> Self {
        let state = app.state::<AppState>();
        let (id, watcher, pump) = {
            let mut slot = lock(&state.blackbox);
            match slot.as_mut() {
                Some(b) => {
                    b.pauses += 1;
                    (Some(b.id), b.watcher.take(), b.pump.take())
                }
                None => (None, None, None),
            }
        };
        if let Some(w) = watcher {
            tracing::info!("чорна скринька: пауза на час запису");
            w.stop();
            if let Some(p) = pump {
                let _ = p.join();
            }
            // `Ended` міг прийти вже після `take` — час паузи рахуємо від цього моменту.
            if let Some(b) = lock(&state.blackbox).as_mut().filter(|b| Some(b.id) == id) {
                b.played_before_s += b.played_now_s;
                b.played_now_s = 0.0;
                b.paused_since.get_or_insert_with(Instant::now);
            }
            emit(app);
        }
        Self {
            app: app.clone(),
            id,
        }
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        let mut slot = lock(&state.blackbox);
        if let Some(b) = slot.as_mut().filter(|b| Some(b.id) == self.id) {
            b.pauses = b.pauses.saturating_sub(1);
            b.resume_after = Some(Instant::now() + RESUME_DELAY);
        }
    }
}

/// Стан чорної скриньки: поточна сесія й остання збережена.
#[tauri::command]
pub async fn blackbox_state(app: AppHandle) -> CmdResult<BlackBoxView> {
    super::blocking(move || Ok(view(&app.state::<AppState>()))).await
}
