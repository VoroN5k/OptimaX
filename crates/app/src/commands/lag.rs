//! «Щойно лагнуло»: стеження за грою з кільцевим буфером останніх 30 с. `Ctrl+Shift+F11` у грі
//! (або кнопка) зберігає знімок, який аналізується так само, як звичайний запис.
//! Події: `lag-status` (щосекунди, [`WatchStatus`]), `lag-update` ([`LagView`] — новий знімок,
//! невдалий знімок або кінець стеження). Нічого в системі не змінює.

use super::diagnosis::{context, shader_cache_files};
use crate::dto::{DiagnosisResult, LagCapture, LagView};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_core::Catalog;
use optimax_diagnosis::diagnose;
use optimax_frametime::watch::{
    CaptureFailure, WatchEnd, WatchEvent, WatchNotices, WatchOptions, WatchStatus, Watcher,
};
use optimax_frametime::{EndReason, Target};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use std::sync::mpsc::Receiver;
use tauri::{AppHandle, Emitter, Manager, State};

/// Скільки знімків тримати за сесію (старіші відкидаються).
const MAX_CAPTURES: usize = 10;

/// Сесія стеження (лишається після зупинки, щоб знімки можна було переглянути).
pub struct LagSession {
    /// Номер сесії: події старої сесії після перезапуску ігноруються.
    id: u64,
    game: String,
    window_s: u32,
    watcher: Option<Watcher>,
    status: Option<WatchStatus>,
    captures: Vec<LagCapture>,
    next_capture: u32,
    last_failure: Option<CaptureFailure>,
    ended: Option<WatchEnd>,
}

impl LagSession {
    fn view(&self) -> LagView {
        LagView {
            game: self.game.clone(),
            window_s: self.window_s,
            watching: self.watcher.is_some() && self.ended.is_none(),
            status: self.status,
            captures: self.captures.clone(),
            last_failure: self.last_failure,
            ended: self.ended.clone(),
        }
    }
}

/// Чи йде стеження (тоді інші записи гри не запускаються: PresentMon і трасування DPC
/// мають по одній сесії на систему).
pub fn is_watching(state: &AppState) -> bool {
    lock(&state.lag)
        .as_ref()
        .is_some_and(|s| s.watcher.is_some() && s.ended.is_none())
}

/// Помилка «іде стеження» для інших команд запису.
pub fn busy_error() -> CmdError {
    CmdError::Failed("іде стеження «Щойно лагнуло» — спершу зупини його".into())
}

/// Починає стеження за грою.
#[tauri::command]
pub fn lag_start(app: AppHandle, state: State<'_, AppState>, exe: String) -> CmdResult<LagView> {
    let target = Target::ProcessName(exe.clone());
    target.validate()?;
    if lock(&state.recording).is_some()
        || lock(&state.ab).is_some()
        || super::resolution::is_running(&state)
    {
        return Err(CmdError::Failed(
            "уже йде запис, A/B-тест або тест роздільності — дочекайся кінця".into(),
        ));
    }
    if is_watching(&state) {
        return Err(CmdError::Failed("стеження вже йде".into()));
    }
    // Попередня (зупинена) сесія закривається разом зі знімками.
    stop_session(&state);

    let sound = lock(&state.settings).record_sound;
    let mut options = WatchOptions::new(target);
    options.sound = sound;
    if let Ok(c) = Catalog::ukrainian() {
        let secs = options.window.as_secs();
        options.notices = Some(WatchNotices {
            saving: c.render(&optimax_core::Text::new("toast.lag.saving").arg("s", secs)),
            too_short: c.render(&optimax_core::Text::new("toast.lag.too_short")),
        });
    }
    let window_s = u32::try_from(options.window.as_secs()).unwrap_or(30);
    let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
    let (watcher, events) = Watcher::start(
        state.presentmon()?,
        options,
        Box::new(SysinfoSource::new()),
        gpu,
    )?;

    let mut slot = lock(&state.lag);
    let id = slot.as_ref().map_or(1, |s| s.id + 1);
    let session = LagSession {
        id,
        game: exe,
        window_s,
        watcher: Some(watcher),
        status: None,
        captures: Vec::new(),
        next_capture: 1,
        last_failure: None,
        ended: None,
    };
    let view = session.view();
    *slot = Some(session);
    drop(slot);

    std::thread::Builder::new()
        .name("optimax-lag-events".into())
        .spawn(move || pump(&app, id, &events))?;
    tracing::info!("стеження «Щойно лагнуло» за {} почато", view.game);
    Ok(view)
}

/// Обробляє події стеження до його кінця.
fn pump(app: &AppHandle, id: u64, events: &Receiver<WatchEvent>) {
    let state = app.state::<AppState>();
    let same = |s: &Option<LagSession>| s.as_ref().is_some_and(|s| s.id == id);
    for event in events {
        match event {
            WatchEvent::Status(status) => {
                let mut slot = lock(&state.lag);
                if !same(&slot) {
                    continue;
                }
                if let Some(s) = slot.as_mut() {
                    s.status = Some(status);
                }
                drop(slot);
                let _ = app.emit("lag-status", status);
            }
            WatchEvent::Pressed => {
                let _ = app.emit("lag-pressed", ());
            }
            WatchEvent::Captured {
                recording,
                at_unix_ms,
            } => {
                // Аналіз — поза замком: аудит конфігурації займає до кількох секунд.
                let (mut ctx, display_hz) = context();
                if let Some(start) = recording.start_unix_ms {
                    ctx.shader_cache_files = shader_cache_files(start);
                }
                let result = DiagnosisResult {
                    diagnosis: diagnose(&recording, &ctx),
                    display_hz,
                    presentmon_messages: recording.presentmon_messages.clone(),
                    cancelled: false,
                    counted_s: recording.counted_seconds(),
                    end_reason: EndReason::Completed,
                };
                let mut slot = lock(&state.lag);
                if !same(&slot) {
                    continue;
                }
                if let Some(s) = slot.as_mut() {
                    let capture = LagCapture {
                        id: s.next_capture,
                        at_unix_ms,
                        result,
                    };
                    s.next_capture += 1;
                    s.captures.insert(0, capture);
                    s.captures.truncate(MAX_CAPTURES);
                    s.last_failure = None;
                    tracing::info!("«Щойно лагнуло»: знімок {} збережено", s.game);
                    let _ = app.emit("lag-update", s.view());
                }
            }
            WatchEvent::CaptureFailed { reason, .. } => {
                let mut slot = lock(&state.lag);
                if let Some(s) = slot.as_mut().filter(|s| s.id == id) {
                    s.last_failure = Some(reason);
                    tracing::warn!("«Щойно лагнуло»: знімок не вдався ({reason:?})");
                    let _ = app.emit("lag-update", s.view());
                }
            }
            WatchEvent::Ended(end) => {
                let mut slot = lock(&state.lag);
                if let Some(s) = slot.as_mut().filter(|s| s.id == id) {
                    if let WatchEnd::PresentMonFailed(m) = &end {
                        tracing::error!("«Щойно лагнуло»: PresentMon не працює: {m}");
                    }
                    s.ended = Some(end);
                    // Потік стеження вже завершився — звільняємо його без очікування.
                    let watcher = s.watcher.take();
                    let _ = app.emit("lag-update", s.view());
                    drop(slot);
                    drop(watcher);
                }
                return;
            }
        }
    }
}

/// Поточний стан стеження (якщо сесія є).
#[tauri::command]
pub fn lag_state(state: State<'_, AppState>) -> Option<LagView> {
    lock(&state.lag).as_ref().map(LagSession::view)
}

/// Зберегти останні секунди просто зараз (як гаряча клавіша).
#[tauri::command]
pub fn lag_capture(state: State<'_, AppState>) -> CmdResult<()> {
    match lock(&state.lag).as_ref().and_then(|s| s.watcher.as_ref()) {
        Some(w) => {
            w.capture_now();
            Ok(())
        }
        None => Err(CmdError::NotFound("стеження не запущено".into())),
    }
}

/// Зупиняє стеження (знімки лишаються до наступного старту або закриття).
#[tauri::command]
pub async fn lag_stop(app: AppHandle) -> CmdResult<Option<LagView>> {
    super::blocking(move || {
        let state = app.state::<AppState>();
        let watcher = lock(&state.lag).as_mut().and_then(|s| s.watcher.take());
        if let Some(w) = watcher {
            w.stop();
        }
        let mut slot = lock(&state.lag);
        if let Some(s) = slot.as_mut() {
            if s.ended.is_none() {
                s.ended = Some(WatchEnd::Stopped);
            }
        }
        Ok(slot.as_ref().map(LagSession::view))
    })
    .await
}

/// Закриває сесію разом зі знімками.
#[tauri::command]
pub async fn lag_close(app: AppHandle) -> CmdResult<()> {
    super::blocking(move || {
        stop_session(&app.state::<AppState>());
        Ok(())
    })
    .await
}

/// Зупиняє стеження й прибирає сесію (виклик із коду: новий старт, вихід із програми).
pub fn stop_session(state: &AppState) {
    let session = lock(&state.lag).take();
    if let Some(w) = session.and_then(|mut s| s.watcher.take()) {
        w.stop();
    }
}
