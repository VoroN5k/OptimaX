//! A/B-тест зміни з інтерфейсу: старт → прогони по одному (користувач готує сцену) → підсумок.
//!
//! Предмет тесту — або один твік, або профіль гри цілком (усе, що профіль робить у режимі
//! гри). Профіль перевіряється на запущеній грі; режим гри на час тесту має бути зупинений,
//! інакше він сам увімкне профіль і в прогонах «без профілю».
//!
//! Статистика — рушій порівнянь (`optimax_experiment`): порядок ABBA, поправка на дрейф,
//! адаптивна кількість прогонів (3–8 на варіант). Варіант 0 (A) — без зміни, 1 (B) — зі зміною.

use super::{blocking, record_game};
use crate::dto::{AbFinal, AbView, UiDesired};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState, FlagConfirm, MEMORY, PROCESSES};
use optimax_core::process::{ProcessApi, ProcessInfo};
use optimax_core::{Catalog, CivilDate, Text};
use optimax_experiment::report::render_markdown;
use optimax_experiment::{evaluate, next_variant, Outcome, Policy, Report, Run, Status, Variant};
use optimax_frametime::{CancelToken, StartMode, Target};
use optimax_gamemode::{
    end_session, preview_changes, session_plan, start_session, GameProfile, ProfileStore,
    ProfileTest, SessionReport,
};
use optimax_tweaks::{Changes, Desired, Plan, TweakError, TweakId};

/// Без зміни.
const OFF: Variant = 0;
/// Зі зміною.
const ON: Variant = 1;
use tauri::{AppHandle, Manager, State};

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
    seconds: u32,
    /// Спосіб старту — однаковий для всіх прогонів тесту.
    start: StartMode,
    sound: bool,
    policy: Policy,
    runs: Vec<Run>,
    report: Report,
    /// Скасування поточного прогону.
    cancel: Option<CancelToken>,
}

impl AbSession {
    fn new(game: String, subject: AbSubject, plan: Plan, seconds: u32, state: &AppState) -> Self {
        let (start, sound) = {
            let st = lock(&state.settings);
            (st.record_start, st.record_sound)
        };
        let policy = Policy::default();
        Self {
            game,
            subject,
            plan,
            seconds: seconds.clamp(20, 300),
            start,
            sound,
            policy,
            runs: Vec::new(),
            report: evaluate(&[], &policy),
            cancel: None,
        }
    }

    fn view(&self) -> AbView {
        AbView {
            game: self.game.clone(),
            plan: self.plan.clone(),
            seconds: self.seconds,
            start: self.start,
            report: self.report.clone(),
            max_per_arm: u32::try_from(self.policy.max_per_arm).unwrap_or(u32::MAX),
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

/// Починає A/B: перевіряє, що зміна не порожня. Порядок і кількість прогонів веде рушій.
#[tauri::command]
pub fn ab_start(
    state: State<'_, AppState>,
    game: String,
    desired: UiDesired,
    seconds: u32,
) -> CmdResult<AbView> {
    Target::ProcessName(game.clone()).validate()?;
    if super::lag::is_watching(&state) {
        return Err(super::lag::busy_error());
    }
    if super::experiment::is_active(&state) {
        return Err(super::experiment::busy_error());
    }
    if super::stability::is_active(&state) {
        return Err(super::stability::busy_error());
    }
    if super::resolution::is_running(&state) {
        return Err(CmdError::Failed(
            "іде прогін тесту роздільності — дочекайся кінця або скасуй тест".into(),
        ));
    }
    let mut slot = lock(&state.ab);
    if slot.is_some() {
        return Err(CmdError::Failed(
            "A/B-тест уже йде — заверши або скасуй його".into(),
        ));
    }
    let desired: Desired = desired.into();
    let plan = lock(&state.changes).plan(&desired)?;
    if plan.is_noop() {
        return Err(CmdError::Failed(
            "система вже у стані «після» — порівнювати нічого".into(),
        ));
    }
    let session = AbSession::new(
        game,
        AbSubject::Tweak {
            desired,
            active_entry: None,
        },
        plan,
        seconds,
        &state,
    );
    let view = session.view();
    *slot = Some(session);
    Ok(view)
}

/// Починає A/B профілю гри: без профілю (A) і з профілем (B), гра має бути запущена.
#[tauri::command]
pub fn ab_start_profile(
    state: State<'_, AppState>,
    exe: String,
    seconds: u32,
) -> CmdResult<AbView> {
    Target::ProcessName(exe.clone()).validate()?;
    if lock(&state.gamemode).is_some() {
        return Err(CmdError::Failed(
            "зупини режим гри на час перевірки: він сам вмикає профіль і зіпсує прогони без профілю"
                .into(),
        ));
    }
    if super::lag::is_watching(&state) {
        return Err(super::lag::busy_error());
    }
    if super::experiment::is_active(&state) {
        return Err(super::experiment::busy_error());
    }
    if super::stability::is_active(&state) {
        return Err(super::stability::busy_error());
    }
    if super::resolution::is_running(&state) {
        return Err(CmdError::Failed(
            "іде прогін тесту роздільності — дочекайся кінця або скасуй тест".into(),
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
    let changes = preview_changes(&mut *lock(&state.changes), &session_plan);
    if changes.is_empty() {
        return Err(CmdError::Failed(
            "профіль зараз нічого не змінює для цієї гри — порівнювати нічого".into(),
        ));
    }
    let session = AbSession::new(
        profile.exe.clone(),
        AbSubject::Profile {
            profile: Box::new(profile),
            session: None,
        },
        Plan {
            info: TweakId::GameProfile.info(),
            changes,
        },
        seconds,
        &state,
    );
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
            if s.report.status == Status::Done {
                return Err(CmdError::Failed(
                    "висновок уже є — прогонів більше не треба".into(),
                ));
            }
            if s.cancel.is_some() {
                return Err(CmdError::Failed("прогін уже йде".into()));
            }
            let arm = next_variant(&s.runs, s.policy.variants);
            let mut engine = lock(&state.changes);
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
                    ON,
                ) if active_entry.is_none() => {
                    let plan = engine.plan(desired)?;
                    match engine.apply(&plan, &confirm) {
                        Ok(out) => *active_entry = out.entry_ids.last().copied(),
                        Err(TweakError::Cancelled) => return Err(cancelled(&confirm)),
                        Err(e) => return Err(e.into()),
                    }
                }
                (AbSubject::Tweak { active_entry, .. }, OFF) => {
                    if let Some(id) = active_entry.take() {
                        engine.rollback(id)?;
                    }
                }
                (AbSubject::Profile { profile, session }, ON) if session.is_none() => {
                    // Гру могли перезапустити між прогонами — шукаємо процес щоразу.
                    let game = running_game(&profile.exe)?;
                    match start_session(&mut *engine, &PROCESSES, &MEMORY, profile, &game, &confirm)
                    {
                        Ok(report) => *session = Some(report),
                        Err(TweakError::Cancelled) => return Err(cancelled(&confirm)),
                        Err(e) => return Err(e.into()),
                    }
                }
                (AbSubject::Profile { session, .. }, OFF) => {
                    if let Some(report) = session.take() {
                        end_session(&mut *engine, &report);
                    }
                }
                _ => {}
            }
            let cancel = CancelToken::new();
            s.cancel = Some(cancel.clone());
            (arm, s.game.clone(), s.seconds, s.start, s.sound, cancel)
        };
        // 2. Запис (довго, без замків).
        let measured = record_game(&app, game, seconds, start, sound, &cancel, "ab-progress")
            .and_then(|rec| {
                if rec.cancelled {
                    return Err(CmdError::Cancelled("прогін зупинено".into()));
                }
                super::experiment::run_from(&rec, arm)
            });
        // 3. Зберегти результат і перерахувати висновок.
        let mut guard = lock(&state.ab);
        let s = guard
            .as_mut()
            .ok_or_else(|| CmdError::NotFound("A/B-тест скасовано під час прогону".into()))?;
        s.cancel = None;
        s.runs.push(measured?);
        s.report = evaluate(&s.runs, &s.policy);
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
    if s.report.status != Status::Done {
        return Err(CmdError::Failed(
            "висновку ще немає — потрібні ще прогони".into(),
        ));
    }
    let report = s.report.clone();
    let catalog = Catalog::ukrainian()?;
    let title = catalog.render(&s.plan.info.title);
    let names = [
        catalog.render(&Text::new("ui.opt.arm_a")),
        catalog.render(&Text::new("ui.opt.arm_b")),
    ];
    let markdown = render_markdown(&report, &catalog, &names, &title, &s.game, s.seconds);
    let cmp = report.comparison(ON);
    let effect = cmp.map_or(Outcome::Unclear, |c| c.verdict);
    let kept = match &s.subject {
        AbSubject::Tweak { active_entry, .. } => {
            // Залишаємо лише виміряний приріст (CLAUDE.md, розд. 6).
            let kept = keep && effect == Outcome::Better;
            if let (false, Some(id)) = (kept, *active_entry) {
                lock(&state.changes).rollback(id)?;
            }
            kept
        }
        AbSubject::Profile { profile, session } => {
            // Профіль сам діє лише в режимі гри: після тесту все повертаємо, а результат
            // зберігаємо в профіль, щоб його було видно на екрані «Ігри».
            if let Some(report) = session {
                end_session(&mut *lock(&state.changes), report);
            }
            let mut updated = (**profile).clone();
            let delta = |e: Option<&optimax_experiment::Estimate>| e.map_or(0.0, |e| e.delta_pct);
            updated.last_test = Some(ProfileTest {
                date: CivilDate::today(),
                runs_per_arm: u32::try_from(report.runs_of(OFF).min(report.runs_of(ON)))
                    .unwrap_or(0),
                avg_fps_pct: delta(cmp.and_then(|c| c.avg_fps.as_ref())),
                low_1pct_pct: delta(cmp.and_then(|c| c.low_1pct.as_ref())),
                effect,
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
            } => lock(&state.changes).rollback(*id)?,
            AbSubject::Profile {
                session: Some(report),
                ..
            } => {
                end_session(&mut *lock(&state.changes), report);
            }
            _ => {}
        }
    }
    *guard = None;
    Ok(())
}
