//! A/B-тест зміни з інтерфейсу: старт → прогони по одному (користувач готує сцену) → підсумок.
//!
//! Предмет тесту — або один твік, або профіль гри цілком (усе, що профіль робить у режимі
//! гри). Профіль перевіряється на запущеній грі; режим гри на час тесту має бути зупинений,
//! інакше він сам увімкне профіль і в прогонах «без профілю».

use super::blocking;
use crate::dto::{AbFinal, AbView, UiDesired};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState, FlagConfirm, MEMORY, PROCESSES};
use optimax_core::process::{ProcessApi, ProcessInfo};
use optimax_core::{Catalog, CivilDate};
use optimax_diagnosis::{diagnose, Bottleneck, Context};
use optimax_frametime::{
    record_with, CancelToken, RecordNotices, RecordOptions, StartMode, Target,
};
use optimax_gamemode::{
    end_session, preview_changes, session_plan, start_session, GameProfile, ProfileStore,
    ProfileTest, SessionReport,
};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use optimax_tweaks::ab::{
    evaluate, render_markdown, schedule, Arm, Decision, RunConditions, RunResult,
};
use optimax_tweaks::{Desired, Plan, TweakError, TweakId};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Що перевіряє A/B.
enum AbSubject {
    /// Один твік; `active_entry` — запис журналу, поки зміна застосована.
    Tweak {
        desired: Desired,
        active_entry: Option<u64>,
    },
    /// Профіль гри; `session` — поки профіль застосований.
    Profile {
        profile: Box<GameProfile>,
        session: Option<SessionReport>,
    },
}

/// Сесія A/B.
pub struct AbSession {
    game: String,
    subject: AbSubject,
    plan: Plan,
    order: Vec<Arm>,
    next: usize,
    seconds: u32,
    /// Спосіб старту — однаковий для всіх прогонів тесту.
    start: StartMode,
    sound: bool,
    results: Vec<RunResult>,
    /// Скасування поточного прогону.
    cancel: Option<CancelToken>,
}

impl AbSession {
    fn view(&self) -> AbView {
        AbView {
            game: self.game.clone(),
            plan: self.plan.clone(),
            order: self.order.clone(),
            next: u32::try_from(self.next).unwrap_or(u32::MAX),
            seconds: self.seconds,
            start: self.start,
            results: self.results.clone(),
            profile: matches!(self.subject, AbSubject::Profile { .. }),
        }
    }
}

fn profiles() -> CmdResult<ProfileStore> {
    ProfileStore::default_location().map_err(|e| CmdError::Failed(e.to_string()))
}

/// Запущений процес гри (перший за іменем).
fn running_game(exe: &str) -> CmdResult<ProcessInfo> {
    PROCESSES
        .find_by_name(exe)
        .map_err(|e| CmdError::Failed(e.to_string()))?
        .into_iter()
        .next()
        .ok_or_else(|| {
            CmdError::NotFound(format!(
                "{exe} не запущено — запусти гру: профіль перевіряється на запущеній грі"
            ))
        })
}

/// Починає A/B: перевіряє, що зміна не порожня, і складає розклад ABAB…
#[tauri::command]
pub fn ab_start(
    state: State<'_, AppState>,
    game: String,
    desired: UiDesired,
    runs: u32,
    seconds: u32,
) -> CmdResult<AbView> {
    Target::ProcessName(game.clone()).validate()?;
    let mut slot = lock(&state.ab);
    if slot.is_some() {
        return Err(CmdError::Failed(
            "A/B-тест уже йде — заверши або скасуй його".into(),
        ));
    }
    let desired: Desired = desired.into();
    let plan = lock(&state.engine).plan(&desired)?;
    if plan.is_noop() {
        return Err(CmdError::Failed(
            "система вже у стані «після» — порівнювати нічого".into(),
        ));
    }
    let (start, sound) = {
        let st = lock(&state.settings);
        (st.record_start, st.record_sound)
    };
    let session = AbSession {
        start,
        sound,
        game,
        subject: AbSubject::Tweak {
            desired,
            active_entry: None,
        },
        plan,
        order: schedule(usize::try_from(runs).unwrap_or(3)),
        next: 0,
        seconds: seconds.clamp(20, 300),
        results: Vec::new(),
        cancel: None,
    };
    let view = session.view();
    *slot = Some(session);
    Ok(view)
}

/// Починає A/B профілю гри: без профілю (A) і з профілем (B), гра має бути запущена.
#[tauri::command]
pub fn ab_start_profile(
    state: State<'_, AppState>,
    exe: String,
    runs: u32,
    seconds: u32,
) -> CmdResult<AbView> {
    Target::ProcessName(exe.clone()).validate()?;
    if lock(&state.gamemode).is_some() {
        return Err(CmdError::Failed(
            "зупини режим гри на час перевірки: він сам вмикає профіль і зіпсує прогони без профілю"
                .into(),
        ));
    }
    let mut slot = lock(&state.ab);
    if slot.is_some() {
        return Err(CmdError::Failed(
            "A/B-тест уже йде — заверши або скасуй його".into(),
        ));
    }
    let profile = profiles()?
        .load()?
        .into_iter()
        .find(|p| p.exe.eq_ignore_ascii_case(&exe))
        .ok_or_else(|| CmdError::NotFound(format!("немає профілю для {exe}")))?;
    let game = running_game(&profile.exe)?;
    let session_plan = session_plan(&PROCESSES, &profile, &game)?;
    let changes = preview_changes(&lock(&state.engine), &session_plan);
    if changes.is_empty() {
        return Err(CmdError::Failed(
            "профіль зараз нічого не змінює для цієї гри — порівнювати нічого".into(),
        ));
    }
    let (start, sound) = {
        let st = lock(&state.settings);
        (st.record_start, st.record_sound)
    };
    let session = AbSession {
        start,
        sound,
        game: profile.exe.clone(),
        subject: AbSubject::Profile {
            profile: Box::new(profile),
            session: None,
        },
        plan: Plan {
            info: TweakId::GameProfile.info(),
            changes,
        },
        order: schedule(usize::try_from(runs).unwrap_or(3)),
        next: 0,
        seconds: seconds.clamp(20, 300),
        results: Vec::new(),
        cancel: None,
    };
    let view = session.view();
    *slot = Some(session);
    Ok(view)
}

/// Поточний стан A/B (якщо є).
#[tauri::command]
pub fn ab_state(state: State<'_, AppState>) -> Option<AbView> {
    lock(&state.ab).as_ref().map(AbSession::view)
}

/// Виконує наступний прогін: ставить потрібний варіант, записує гру, рахує FPS.
#[tauri::command]
pub async fn ab_run_next(app: AppHandle, allow_without_restore_point: bool) -> CmdResult<AbView> {
    blocking(move || {
        let state = app.state::<AppState>();
        // 1. Привести систему до потрібного варіанта (під замком сесії, недовго).
        let (arm, game, seconds, start, sound, cancel) = {
            let mut guard = lock(&state.ab);
            let s = guard
                .as_mut()
                .ok_or_else(|| CmdError::NotFound("A/B-тест не запущено".into()))?;
            let arm = *s
                .order
                .get(s.next)
                .ok_or_else(|| CmdError::Failed("усі прогони вже виконано".into()))?;
            let mut engine = lock(&state.engine);
            let confirm = FlagConfirm::new(allow_without_restore_point);
            let cancelled = |confirm: &FlagConfirm| {
                confirm.reason().map_or(
                    CmdError::Cancelled("скасовано".into()),
                    CmdError::RestorePoint,
                )
            };
            match (&mut s.subject, arm) {
                (
                    AbSubject::Tweak {
                        desired,
                        active_entry,
                    },
                    Arm::B,
                ) if active_entry.is_none() => {
                    let plan = engine.plan(desired)?;
                    match engine.apply(&plan, &confirm) {
                        Ok(out) => *active_entry = out.entry_ids.last().copied(),
                        Err(TweakError::Cancelled) => return Err(cancelled(&confirm)),
                        Err(e) => return Err(e.into()),
                    }
                }
                (AbSubject::Tweak { active_entry, .. }, Arm::A) => {
                    if let Some(id) = active_entry.take() {
                        engine.rollback(id)?;
                    }
                }
                (AbSubject::Profile { profile, session }, Arm::B) if session.is_none() => {
                    // Гру могли перезапустити між прогонами — шукаємо процес щоразу.
                    let game = running_game(&profile.exe)?;
                    match start_session(&mut engine, &PROCESSES, &MEMORY, profile, &game, &confirm)
                    {
                        Ok(report) => *session = Some(report),
                        Err(TweakError::Cancelled) => return Err(cancelled(&confirm)),
                        Err(e) => return Err(e.into()),
                    }
                }
                (AbSubject::Profile { session, .. }, Arm::A) => {
                    if let Some(report) = session.take() {
                        end_session(&mut engine, &report);
                    }
                }
                _ => {}
            }
            let cancel = CancelToken::new();
            s.cancel = Some(cancel.clone());
            (arm, s.game.clone(), s.seconds, s.start, s.sound, cancel)
        };
        // 2. Запис (довго, без замків).
        let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
        let mut options = RecordOptions::new(
            Target::ProcessName(game),
            Duration::from_secs(u64::from(seconds)),
        )
        .with_start(start, sound);
        if let Ok(catalog) = Catalog::ukrainian() {
            options = options.with_notices(RecordNotices::from_catalog(&catalog));
        }
        let rec = record_with(
            state.presentmon()?,
            &options,
            Box::new(SysinfoSource::new()),
            gpu,
            &cancel,
            &mut |p| {
                let _ = app.emit("ab-progress", p);
            },
        )?;
        if rec.cancelled {
            return Err(CmdError::Cancelled("прогін зупинено".into()));
        }
        let d = diagnose(&rec, &Context::default());
        let stats = d
            .stats
            .ok_or_else(|| CmdError::Failed("у прогоні немає кадрів гри".into()))?;
        let temps: Vec<f64> = rec
            .snapshots
            .iter()
            .filter_map(|s| s.gpus.iter().filter_map(|g| g.temperature_c).max())
            .map(f64::from)
            .collect();
        let result = RunResult {
            arm,
            avg_fps: stats.avg_fps,
            low_1pct_fps: stats.low_1pct_fps,
            conditions: RunConditions {
                duration_s: stats.duration_ms / 1000.0,
                frames: stats.frames,
                gpu_temp_avg_c: (!temps.is_empty())
                    .then(|| temps.iter().sum::<f64>() / temps.len() as f64),
                frame_capped: d.bottleneck == Bottleneck::FrameCap,
            },
        };
        // 3. Зберегти результат.
        let mut guard = lock(&state.ab);
        let s = guard
            .as_mut()
            .ok_or_else(|| CmdError::NotFound("A/B-тест скасовано під час прогону".into()))?;
        s.results.push(result);
        s.next += 1;
        s.cancel = None;
        Ok(s.view())
    })
    .await
}

/// Підсумок. Якщо зміна дала реальний приріст і `keep` — залишаємо, інакше відкочуємо.
#[tauri::command]
pub fn ab_finish(state: State<'_, AppState>, keep: bool) -> CmdResult<AbFinal> {
    let mut guard = lock(&state.ab);
    let s = guard
        .as_ref()
        .ok_or_else(|| CmdError::NotFound("A/B-тест не запущено".into()))?;
    let report = evaluate(&s.results)?;
    let catalog = Catalog::ukrainian()?;
    let title = catalog.render(&s.plan.info.title);
    let markdown = render_markdown(&report, &catalog, &title, &s.game);
    let kept = match &s.subject {
        AbSubject::Tweak { active_entry, .. } => {
            let kept = keep && report.decision == Decision::Keep;
            if let (false, Some(id)) = (kept, *active_entry) {
                lock(&state.engine).rollback(id)?;
            }
            kept
        }
        AbSubject::Profile { profile, session } => {
            // Профіль сам діє лише в режимі гри: після тесту все повертаємо, а результат
            // зберігаємо в профіль, щоб його було видно на екрані «Ігри».
            if let Some(report) = session {
                end_session(&mut lock(&state.engine), report);
            }
            let mut updated = (**profile).clone();
            updated.last_test = Some(ProfileTest {
                date: CivilDate::today(),
                runs_per_arm: u32::try_from(s.results.len() / 2).unwrap_or(0),
                avg_fps_pct: report.avg_fps.delta_pct,
                low_1pct_pct: report.low_1pct.delta_pct,
                effect: report.effect,
            });
            profiles()?.upsert(&updated)?;
            false
        }
    };
    *guard = None;
    Ok(AbFinal {
        report,
        markdown,
        kept,
    })
}

/// Скасовує тест: зупиняє прогін і відкочує зміну, якщо вона застосована.
#[tauri::command]
pub fn ab_cancel(state: State<'_, AppState>) -> CmdResult<()> {
    cancel_session(&state)
}

/// Те саме, що [`ab_cancel`], для виклику з коду (вихід із програми).
pub fn cancel_session(state: &AppState) -> CmdResult<()> {
    let mut guard = lock(&state.ab);
    if let Some(s) = guard.as_ref() {
        if let Some(c) = &s.cancel {
            c.cancel();
        }
        match &s.subject {
            AbSubject::Tweak {
                active_entry: Some(id),
                ..
            } => lock(&state.engine).rollback(*id)?,
            AbSubject::Profile {
                session: Some(report),
                ..
            } => {
                end_session(&mut lock(&state.engine), report);
            }
            _ => {}
        }
    }
    *guard = None;
    Ok(())
}
