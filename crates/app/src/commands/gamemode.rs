//! Режим гри: профілі та фоновий стежник (подія `gamemode`).

use crate::dto::{GameModeEvent, ProfilePreview, RollbackResult};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState, FlagConfirm, MEMORY, PROCESSES};
use optimax_core::process::ProcessInfo;
use optimax_core::Text;
use optimax_gamemode::watcher::POLL_INTERVAL;
use optimax_gamemode::{
    end_session, scan, start_session, step, GameProfile, ProfileStore, SessionReport, WatchEvent,
    WatchState,
};
use optimax_tweaks::{Engine, TweakError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// Запущений стежник.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Watcher {
    /// Зупиняє стежник (активну сесію буде повернуто) і чекає завершення.
    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn store() -> CmdResult<ProfileStore> {
    ProfileStore::default_location().map_err(|e| CmdError::Failed(e.to_string()))
}

/// Профілі ігор.
#[tauri::command]
pub fn profiles_list() -> CmdResult<Vec<GameProfile>> {
    Ok(store()?.load()?)
}

/// Безпечна версія профілю й попередження — для показу перед збереженням.
#[tauri::command]
pub fn profile_preview(profile: GameProfile) -> CmdResult<ProfilePreview> {
    let (profile, warnings) = profile.sanitized()?;
    Ok(ProfilePreview { profile, warnings })
}

/// Зберігає профіль (безпечну версію). Повертає попередження.
#[tauri::command]
pub fn profile_save(profile: GameProfile) -> CmdResult<Vec<Text>> {
    Ok(store()?.upsert(&profile)?)
}

/// Видаляє профіль.
#[tauri::command]
pub fn profile_remove(exe: String) -> CmdResult<bool> {
    Ok(store()?.remove(&exe)?)
}

/// Чи працює стежник.
#[tauri::command]
pub fn gamemode_running(state: State<'_, AppState>) -> bool {
    lock(&state.gamemode).is_some()
}

/// Дозволяє режиму гри працювати без точки відновлення (до кінця цього запуску програми).
#[tauri::command]
pub fn gamemode_allow_without_restore_point(state: State<'_, AppState>) {
    *lock(&state.gamemode_allow_without_restore) = true;
}

/// Запускає стежник за профілями.
#[tauri::command]
pub fn gamemode_start(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let mut slot = lock(&state.gamemode);
    if slot.is_some() {
        return Ok(());
    }
    let profiles = store()?.load()?;
    if profiles.is_empty() {
        return Err(CmdError::Failed(
            "немає профілів ігор — спершу створи профіль".into(),
        ));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let engine = state.engine.clone();
    let allow = state.gamemode_allow_without_restore.clone();
    let handle = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("optimax-gamemode".into())
            .spawn(move || watch_loop(&app, &engine, &allow, &profiles, &stop))?
    };
    *slot = Some(Watcher {
        stop,
        handle: Some(handle),
    });
    Ok(())
}

/// Зупиняє стежник; якщо гра йде — усе повертається.
#[tauri::command]
pub fn gamemode_stop(state: State<'_, AppState>) {
    let watcher = lock(&state.gamemode).take();
    if let Some(w) = watcher {
        w.shutdown();
    }
}

fn finish(app: &AppHandle, engine: &Mutex<Engine<'static>>, game: &str, report: &SessionReport) {
    let result: RollbackResult = end_session(&mut lock(engine), report).into();
    let _ = app.emit(
        "gamemode",
        GameModeEvent::Ended {
            game: game.into(),
            result,
        },
    );
}

fn watch_loop(
    app: &AppHandle,
    engine: &Mutex<Engine<'static>>,
    allow: &Mutex<bool>,
    profiles: &[GameProfile],
    stop: &AtomicBool,
) {
    let _ = app.emit(
        "gamemode",
        GameModeEvent::Watching {
            games: profiles.iter().map(|p| p.exe.clone()).collect(),
        },
    );
    let mut state = WatchState::Idle;
    let mut session: Option<SessionReport> = None;
    // Гра, для якої чекаємо згоди користувача (щоб не питати щопівсекунди).
    let mut waiting_confirm: Option<(ProcessInfo, bool)> = None;
    let mut next_poll = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
        if Instant::now() < next_poll {
            continue;
        }
        next_poll = Instant::now() + POLL_INTERVAL;
        let allowed = *lock(allow);
        match step(&state, &scan(&PROCESSES, profiles)) {
            Some(WatchEvent::GameStarted { profile_exe, game }) => {
                if let Some((g, was_allowed)) = &waiting_confirm {
                    if g == &game && *was_allowed == allowed {
                        continue;
                    }
                }
                let Some(profile) = profiles.iter().find(|p| p.exe == profile_exe) else {
                    continue;
                };
                let confirm = FlagConfirm::new(allowed);
                let result = start_session(
                    &mut lock(engine),
                    &PROCESSES,
                    &MEMORY,
                    profile,
                    &game,
                    &confirm,
                );
                match result {
                    Ok(report) => {
                        waiting_confirm = None;
                        let _ = app.emit(
                            "gamemode",
                            GameModeEvent::Started {
                                game: game.name.clone(),
                                pid: game.pid,
                                applied: report.applied.clone(),
                                skipped: report.skipped.clone(),
                            },
                        );
                        session = Some(report);
                        state = WatchState::Active { profile_exe, game };
                    }
                    Err(TweakError::Cancelled) => {
                        let reason = confirm.reason().unwrap_or_default();
                        let _ = app.emit(
                            "gamemode",
                            GameModeEvent::NeedsConfirmation {
                                game: game.name.clone(),
                                reason,
                            },
                        );
                        waiting_confirm = Some((game, allowed));
                    }
                    Err(e) => {
                        tracing::error!("режим гри для {} не ввімкнено: {e}", game.name);
                        waiting_confirm = Some((game, allowed));
                    }
                }
            }
            Some(WatchEvent::GameExited) => {
                if let (Some(report), WatchState::Active { game, .. }) = (session.take(), &state) {
                    finish(app, engine, &game.name, &report);
                }
                state = WatchState::Idle;
            }
            None => {
                if matches!(state, WatchState::Idle) {
                    waiting_confirm = None;
                }
            }
        }
    }
    if let (Some(report), WatchState::Active { game, .. }) = (session.take(), &state) {
        finish(app, engine, &game.name, &report);
    }
    let _ = app.emit("gamemode", GameModeEvent::Stopped);
}
