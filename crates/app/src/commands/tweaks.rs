//! Зміни з переглядом «що зміниться», журнал і відкат.

use crate::dto::{ApplyResult, PlanHandle, PowerSchemes, RollbackResult, UiDesired};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState, FlagConfirm, POWER};
use optimax_core::power::PowerApi;
use optimax_tweaks::{JournalEntry, TweakError};
use tauri::State;

/// Схеми живлення в системі й активна схема.
#[tauri::command]
pub fn power_schemes() -> CmdResult<PowerSchemes> {
    Ok(PowerSchemes {
        schemes: POWER.schemes()?,
        active: POWER.active_scheme()?,
    })
}

/// Будує план і запам'ятовує його. Застосувати можна лише план із цим номером.
#[tauri::command]
pub fn tweak_plan(state: State<'_, AppState>, desired: UiDesired) -> CmdResult<PlanHandle> {
    let plan = lock(&state.engine).plan(&desired.into())?;
    let mut next = lock(&state.next_plan_id);
    let id = *next;
    *next += 1;
    lock(&state.plans).insert(id, plan.clone());
    Ok(PlanHandle { id, plan })
}

/// Застосовує показаний план. Якщо точку відновлення не створено, повертає
/// `RestorePoint(причина)`; UI питає користувача й повторює з `allow_without_restore_point`.
#[tauri::command]
pub fn tweak_apply(
    state: State<'_, AppState>,
    plan_id: u64,
    allow_without_restore_point: bool,
) -> CmdResult<ApplyResult> {
    let plan = lock(&state.plans)
        .get(&plan_id)
        .cloned()
        .ok_or_else(|| CmdError::NotFound(format!("план #{plan_id} — переглянь зміни ще раз")))?;
    let confirm = FlagConfirm::new(allow_without_restore_point);
    let result = lock(&state.engine).apply(&plan, &confirm);
    match result {
        Ok(out) => {
            lock(&state.plans).remove(&plan_id);
            Ok(ApplyResult {
                entry_ids: out.entry_ids,
                restore_point: out.restore_point,
            })
        }
        Err(TweakError::Cancelled) => match confirm.reason() {
            Some(reason) => Err(CmdError::RestorePoint(reason)),
            None => Err(CmdError::Cancelled("скасовано".into())),
        },
        Err(e) => Err(e.into()),
    }
}

/// Журнал змін (найновіші — останні).
#[tauri::command]
pub fn journal_entries(state: State<'_, AppState>) -> Vec<JournalEntry> {
    lock(&state.engine).entries().to_vec()
}

/// Відкочує один запис.
#[tauri::command]
pub fn journal_rollback(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    Ok(lock(&state.engine).rollback(id)?)
}

/// Відкочує всі діючі зміни.
#[tauri::command]
pub fn journal_rollback_all(state: State<'_, AppState>) -> RollbackResult {
    lock(&state.engine).rollback_all().into()
}
