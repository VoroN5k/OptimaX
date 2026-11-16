//! Стежник режиму гри без Tauri: бачить гру зі списку профілів, вмикає профіль (через рушій
//! змін — службу або процес), запам'ятовує шлях до exe й вибір відеокарти, веде чорну скриньку,
//! після гри все повертає. Події — через [`Host`] (вікно програми шле їх в інтерфейс, агент у
//! треї — у файл стану й повідомлення поверх гри).

use super::{blackbox, Host, HostRef};
use crate::dto::{GameModeEvent, RollbackResult};
use crate::link::Link;
use crate::profiles;
use crate::state::{lock, FlagConfirm, MEMORY, PROCESSES};
use optimax_core::process::ProcessInfo;
use optimax_core::Text;
use optimax_gamemode::watcher::POLL_INTERVAL;
use optimax_gamemode::{
    end_session, scan, start_session, step, GameProfile, SessionReport, WatchEvent, WatchState,
};
use optimax_tweaks::{Changes, Confirm, Desired, TweakError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Запущений стежник.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    /// Профілі, з якими його запущено.
    profiles: Vec<GameProfile>,
}

impl Watcher {
    /// Запускає стежник за `profiles` у власному потоці.
    pub fn start(host: HostRef, profiles: Vec<GameProfile>) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let stop = stop.clone();
            let profiles = profiles.clone();
            std::thread::Builder::new()
                .name("optimax-gamemode".into())
                .spawn(move || watch_loop(&host, &profiles, &stop))?
        };
        Ok(Self {
            stop,
            handle: Some(handle),
            profiles,
        })
    }

    /// Профілі, з якими працює.
    pub fn profiles(&self) -> &[GameProfile] {
        &self.profiles
    }

    /// Зупиняє стежник (активну сесію буде повернуто) і чекає завершення.
    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Повний шлях до exe запущеного процесу.
fn exe_path_of(pid: u32) -> Option<String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    sys.process(pid)?
        .exe()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Запам'ятовує шлях до гри в профілі й ставить вибрану відеокарту. Windows читає вибір
/// відеокарти під час запуску гри, тож він постійний (не відкочується після сесії) і діє з
/// наступного запуску; запис — у журналі змін («Резервні копії»).
fn remember_path_and_gpu(
    link: &Link,
    engine: &mut dyn Changes,
    profile: &GameProfile,
    game: &ProcessInfo,
    confirm: &dyn Confirm,
) -> Option<Text> {
    let path = exe_path_of(game.pid)?;
    if profile.exe_path.as_deref() != Some(path.as_str()) {
        let mut updated = profile.clone();
        updated.exe_path = Some(path.clone());
        if let Err(e) = profiles::upsert(link, &updated) {
            tracing::warn!(
                "не вдалося запам'ятати шлях до {}: {}",
                profile.exe,
                e.message()
            );
        }
    }
    let preference = profile.gpu?;
    let desired = Desired::GpuPreference {
        exe_path: path,
        preference,
    };
    let plan = engine.plan(&desired).ok()?;
    if plan.is_noop() {
        return None;
    }
    match engine.apply(&plan, confirm) {
        Ok(_) => Some(Text::new("gm.applied.gpu")),
        Err(e) => Some(
            Text::new("gm.skip.error")
                .arg("what", "GPU")
                .arg("error", e),
        ),
    }
}

fn finish(host: &dyn Host, game: &str, report: &SessionReport) {
    let result: RollbackResult = end_session(&mut *lock(&host.core().changes), report).into();
    host.gamemode_event(GameModeEvent::Ended {
        game: game.into(),
        result,
    });
}

fn watch_loop(host: &HostRef, profiles: &[GameProfile], stop: &AtomicBool) {
    let core = host.core();
    host.gamemode_event(GameModeEvent::Watching {
        games: profiles.iter().map(|p| p.exe.clone()).collect(),
    });
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
        let allowed = *lock(&core.allow_without_restore);
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
                    &mut *lock(&core.changes),
                    &PROCESSES,
                    &MEMORY,
                    profile,
                    &game,
                    &confirm,
                );
                match result {
                    Ok(mut report) => {
                        waiting_confirm = None;
                        // Окремо від сесії: вибір відеокарти не відкочується після гри.
                        if let Some(t) = remember_path_and_gpu(
                            &core.link,
                            &mut *lock(&core.changes),
                            profile,
                            &game,
                            &confirm,
                        ) {
                            report.applied.push(t);
                        }
                        host.gamemode_event(GameModeEvent::Started {
                            game: game.name.clone(),
                            pid: game.pid,
                            applied: report.applied.clone(),
                            skipped: report.skipped.clone(),
                        });
                        if profile.black_box {
                            blackbox::start(host, &game.name);
                        }
                        session = Some(report);
                        state = WatchState::Active { profile_exe, game };
                    }
                    Err(TweakError::Cancelled) => {
                        let reason = confirm.reason().unwrap_or_default();
                        host.gamemode_event(GameModeEvent::NeedsConfirmation {
                            game: game.name.clone(),
                            reason,
                        });
                        waiting_confirm = Some((game, allowed));
                    }
                    Err(e) => {
                        tracing::error!("режим гри для {} не ввімкнено: {e}", game.name);
                        waiting_confirm = Some((game, allowed));
                    }
                }
            }
            Some(WatchEvent::GameExited) => {
                blackbox::finish(host.as_ref());
                if let (Some(report), WatchState::Active { game, .. }) = (session.take(), &state) {
                    finish(host.as_ref(), &game.name, &report);
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
    blackbox::finish(host.as_ref());
    if let (Some(report), WatchState::Active { game, .. }) = (session.take(), &state) {
        finish(host.as_ref(), &game.name, &report);
    }
    host.gamemode_event(GameModeEvent::Stopped);
}
