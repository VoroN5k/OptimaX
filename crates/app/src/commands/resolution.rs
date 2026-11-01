//! Тест чутливості FPS до роздільності: два прогони на рідній роздільності, два на зниженій
//! (змінює користувач у грі), підсумок і уточнення останнього діагнозу.
//! Подія `res-progress` — хід поточного прогону. Нічого в системі не змінює.

use super::{blocking, record_game};
use crate::dto::ResView;
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_diagnosis::resolution::{self, ResolutionReport, ResolutionRun};
use optimax_frametime::{CancelToken, StartMode, Target};
use tauri::{AppHandle, Manager, State};

/// Сесія тесту.
pub struct ResSession {
    game: String,
    seconds: u32,
    start: StartMode,
    sound: bool,
    runs: Vec<ResolutionRun>,
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
            order: resolution::schedule().to_vec(),
            runs: self.runs.clone(),
            running: self.cancel.is_some(),
            report: self.report.clone(),
        }
    }
}

/// Починає тест для гри (зазвичай — гри з останнього діагнозу).
#[tauri::command]
pub fn res_start(state: State<'_, AppState>, exe: String, seconds: u32) -> CmdResult<ResView> {
    Target::ProcessName(exe.clone()).validate()?;
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
    let session = ResSession {
        game: exe,
        seconds: seconds.clamp(20, 120),
        start,
        sound,
        runs: Vec::new(),
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

/// Записує наступний прогін; після останнього — підсумок і уточнення діагнозу.
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
            let reduced = *resolution::schedule()
                .get(s.runs.len())
                .ok_or_else(|| CmdError::Failed("усі прогони вже виконано".into()))?;
            let cancel = CancelToken::new();
            s.cancel = Some(cancel.clone());
            (s.game.clone(), s.seconds, s.start, s.sound, reduced, cancel)
        };

        let measured = record_game(&app, game, seconds, start, sound, &cancel, "res-progress")
            .and_then(|rec| {
                if rec.cancelled {
                    return Err(CmdError::Cancelled("прогін зупинено".into()));
                }
                ResolutionRun::from_recording(&rec, reduced)
                    .ok_or_else(|| CmdError::Failed("у прогоні замало кадрів гри".into()))
            });

        let mut guard = lock(&state.res_test);
        let s = guard
            .as_mut()
            .ok_or_else(|| CmdError::NotFound("тест скасовано під час прогону".into()))?;
        s.cancel = None;
        s.runs.push(measured?);
        if let Some(report) = resolution::evaluate(&s.runs) {
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
