//! A/B-тест зміни з інтерфейсу: старт → прогони по одному (користувач готує сцену) → підсумок.

use super::blocking;
use crate::dto::{AbFinal, AbView, UiDesired};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState, FlagConfirm};
use optimax_core::Catalog;
use optimax_diagnosis::{diagnose, Bottleneck, Context};
use optimax_frametime::{record, CancelToken, RecordOptions, Target};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use optimax_tweaks::ab::{
    evaluate, render_markdown, schedule, Arm, Decision, RunConditions, RunResult,
};
use optimax_tweaks::{Desired, Plan, TweakError};
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

/// Сесія A/B.
pub struct AbSession {
    game: String,
    desired: Desired,
    plan: Plan,
    order: Vec<Arm>,
    next: usize,
    seconds: u32,
    results: Vec<RunResult>,
    /// Запис журналу, якщо зміна (варіант B) зараз застосована.
    active_entry: Option<u64>,
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
            results: self.results.clone(),
        }
    }
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
    let session = AbSession {
        game,
        desired,
        plan,
        order: schedule(usize::try_from(runs).unwrap_or(3)),
        next: 0,
        seconds: seconds.clamp(20, 300),
        results: Vec::new(),
        active_entry: None,
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
        let (arm, game, seconds, cancel) = {
            let mut guard = lock(&state.ab);
            let s = guard
                .as_mut()
                .ok_or_else(|| CmdError::NotFound("A/B-тест не запущено".into()))?;
            let arm = *s
                .order
                .get(s.next)
                .ok_or_else(|| CmdError::Failed("усі прогони вже виконано".into()))?;
            let mut engine = lock(&state.engine);
            match (arm, s.active_entry) {
                (Arm::B, None) => {
                    let plan = engine.plan(&s.desired)?;
                    let confirm = FlagConfirm::new(allow_without_restore_point);
                    match engine.apply(&plan, &confirm) {
                        Ok(out) => s.active_entry = out.entry_ids.last().copied(),
                        Err(TweakError::Cancelled) => {
                            return Err(confirm.reason().map_or(
                                CmdError::Cancelled("скасовано".into()),
                                CmdError::RestorePoint,
                            ))
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                (Arm::A, Some(id)) => {
                    engine.rollback(id)?;
                    s.active_entry = None;
                }
                _ => {}
            }
            let cancel = CancelToken::new();
            s.cancel = Some(cancel.clone());
            (arm, s.game.clone(), s.seconds, cancel)
        };
        // 2. Запис (довго, без замків).
        let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
        let rec = record(
            state.presentmon()?,
            &RecordOptions::new(
                Target::ProcessName(game),
                Duration::from_secs(u64::from(seconds)),
            ),
            Box::new(SysinfoSource::new()),
            gpu,
            &cancel,
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
    let kept = keep && report.decision == Decision::Keep;
    if let (false, Some(id)) = (kept, s.active_entry) {
        lock(&state.engine).rollback(id)?;
    }
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
        if let Some(id) = s.active_entry {
            lock(&state.engine).rollback(id)?;
        }
    }
    *guard = None;
    Ok(())
}
