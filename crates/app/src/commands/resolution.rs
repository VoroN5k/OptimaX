//! Тест чутливості FPS до роздільності: прогони по черзі на рідній і зниженій роздільності
//! (змінює користувач у грі) за порядком рушія порівнянь, підсумок і уточнення останнього
//! діагнозу. Подія `res-progress` — хід поточного прогону. Нічого в системі не змінює.

use super::{blocking, record_game};
use crate::dto::ResView;
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_diagnosis::resolution::{self, ResolutionReport, ResolutionRun};
use optimax_diagnosis::Timeline;
use optimax_experiment::{evaluate, next_variant, Policy, Report, Run, Status};
use optimax_frametime::{CancelToken, StartMode, Target};
use tauri::{AppHandle, Manager, State};

/// Сесія тесту.
pub struct ResSession {
    game: String,
    seconds: u32,
    start: StartMode,
    sound: bool,
    policy: Policy,
    /// Прогони для показу (завантаження GPU, ліміт кадрів).
    runs: Vec<ResolutionRun>,
    /// Ті самі прогони для рушія порівнянь.
    exp_runs: Vec<Run>,
    estimate: Report,
    /// Скасування поточного прогону (є, поки прогін іде).
    cancel: Option<CancelToken>,
    report: Option<ResolutionReport>,
}

impl ResSession {
    fn view(&self) -> ResView {
        ResView {
            game: self.game.clone(),
            seconds: self.seconds,
            start: self.start,
            runs: self.runs.clone(),
            next_reduced: (self.estimate.status == Status::Continue)
                .then(|| next_variant(&self.exp_runs, self.policy.variants) == 1),
            max_per_arm: u32::try_from(self.policy.max_per_arm).unwrap_or(u32::MAX),
            estimate: self.estimate.clone(),
            running: self.cancel.is_some(),
            report: self.report.clone(),
        }
    }
}

/// Починає тест для гри (зазвичай — гри з останнього діагнозу).
#[tauri::command]
pub fn res_start(state: State<'_, AppState>, exe: String, seconds: u32) -> CmdResult<ResView> {
    Target::ProcessName(exe.clone()).validate()?;
    if super::lag::is_watching(&state) {
        return Err(super::lag::busy_error());
    }
    if super::experiment::is_active(&state) {
        return Err(super::experiment::busy_error());
    }
    if super::stability::is_active(&state) {
        return Err(super::stability::busy_error());
    }
    if lock(&state.recording).is_some() || lock(&state.ab).is_some() {
        return Err(CmdError::Failed(
            "уже йде запис або A/B-тест — дочекайся кінця".into(),
        ));
    }
    let mut slot = lock(&state.res_test);
    if slot.as_ref().is_some_and(|s| s.cancel.is_some()) {
        return Err(CmdError::Failed("прогін тесту вже йде".into()));
    }
    let (start, sound) = {
        let st = lock(&state.settings);
        (st.record_start, st.record_sound)
    };
    let policy = resolution::policy();
    let session = ResSession {
        game: exe,
        seconds: seconds.clamp(20, 120),
        start,
        sound,
        policy,
        runs: Vec::new(),
        exp_runs: Vec::new(),
        estimate: evaluate(&[], &policy),
        cancel: None,
        report: None,
    };
    let view = session.view();
    *slot = Some(session);
    Ok(view)
}

/// Поточний стан тесту (якщо є).
#[tauri::command]
pub fn res_state(state: State<'_, AppState>) -> Option<ResView> {
    lock(&state.res_test).as_ref().map(ResSession::view)
}

/// Записує наступний прогін; коли висновок певний — підсумок і уточнення діагнозу.
#[tauri::command]
pub async fn res_run_next(app: AppHandle) -> CmdResult<ResView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (game, seconds, start, sound, reduced, cancel) = {
            let mut guard = lock(&state.res_test);
            let s = guard
                .as_mut()
                .ok_or_else(|| CmdError::NotFound("тест роздільності не запущено".into()))?;
            if s.cancel.is_some() {
                return Err(CmdError::Failed("прогін тесту вже йде".into()));
            }
            if super::lag::is_watching(&state) {
                return Err(super::lag::busy_error());
            }
            if s.estimate.status == Status::Done {
                return Err(CmdError::Failed(
                    "висновок уже є — прогонів більше не треба".into(),
                ));
            }
            let reduced = next_variant(&s.exp_runs, s.policy.variants) == 1;
            let cancel = CancelToken::new();
            s.cancel = Some(cancel.clone());
            (s.game.clone(), s.seconds, s.start, s.sound, reduced, cancel)
        };

        let measured = record_game(&app, game, seconds, start, sound, &cancel, "res-progress")
            .and_then(|rec| {
                if rec.cancelled {
                    return Err(CmdError::Cancelled("прогін зупинено".into()));
                }
                let tl = Timeline::from_recording(&rec)
                    .ok_or_else(|| CmdError::Failed("у прогоні немає кадрів гри".into()))?;
                let shown = ResolutionRun::from_timeline(&tl, reduced)
                    .ok_or_else(|| CmdError::Failed("у прогоні замало кадрів гри".into()))?;
                let run = super::experiment::run_from(&rec, u8::from(reduced))?;
                Ok((shown, run))
            });

        let mut guard = lock(&state.res_test);
        let s = guard
            .as_mut()
            .ok_or_else(|| CmdError::NotFound("тест скасовано під час прогону".into()))?;
        s.cancel = None;
        let (shown, run) = measured?;
        s.runs.push(shown);
        s.exp_runs.push(run);
        s.estimate = evaluate(&s.exp_runs, &s.policy);
        if let Some(report) = resolution::evaluate(&s.runs, &s.estimate) {
            if let Some(last) = lock(&state.last_diagnosis).as_mut() {
                if last.diagnosis.game.eq_ignore_ascii_case(&s.game) {
                    resolution::apply(&mut last.diagnosis, &report);
                }
            }
            tracing::info!(
                "тест роздільності {}: {:?}, {:+.1} % FPS",
                s.game,
                report.outcome,
                report.avg_fps_change_pct
            );
            s.report = Some(report);
        }
        Ok(s.view())
    })
    .await
}

/// Чи йде зараз прогін тесту (тоді інші записи гри не запускаються).
pub fn is_running(state: &AppState) -> bool {
    lock(&state.res_test)
        .as_ref()
        .is_some_and(|s| s.cancel.is_some())
}

/// Зупиняє прогін і закриває тест (уточнення діагнозу, якщо вже є, лишається).
#[tauri::command]
pub fn res_cancel(state: State<'_, AppState>) {
    cancel_session(&state);
}

/// Те саме, що [`res_cancel`], для виклику з коду (вихід із програми).
pub fn cancel_session(state: &AppState) {
    if let Some(s) = lock(&state.res_test).take() {
        if let Some(c) = &s.cancel {
            c.cancel();
        }
    }
}
