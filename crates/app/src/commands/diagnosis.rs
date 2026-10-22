//! «Чому лагає?»: запис гри (PresentMon + метрики) і діагноз.
//! Події: `diag-progress` (щосекунди), `diag-done` (результат), `diag-error`.

use super::blocking;
use crate::dto::{DiagnosisResult, ProcessEntry};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_audit::{run_audit, SystemProbe, WindowsProbe};
use optimax_core::{CivilDate, Impact};
use optimax_diagnosis::{diagnose, Context};
use optimax_frametime::{record_with, CancelToken, RecordOptions, Target};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Процеси для вибору гри: унікальні імена, найважчі за CPU зверху.
#[tauri::command]
pub async fn list_processes() -> CmdResult<Vec<ProcessEntry>> {
    blocking(|| {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
        let mut sys = System::new();
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(300)));
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f32;
        let mut by_name: HashMap<String, ProcessEntry> = HashMap::new();
        for p in sys.processes().values() {
            let name = p.name().to_string_lossy().into_owned();
            if !name.to_ascii_lowercase().ends_with(".exe")
                || optimax_core::process::is_protected_process(&name)
            {
                continue;
            }
            let e = by_name.entry(name.clone()).or_insert(ProcessEntry {
                name,
                cpu_percent: 0.0,
                memory_mib: 0,
            });
            e.cpu_percent += p.cpu_usage() / cores;
            e.memory_mib += p.memory() / (1024 * 1024);
        }
        let mut list: Vec<ProcessEntry> = by_name.into_values().collect();
        list.sort_by(|a, b| {
            b.cpu_percent
                .total_cmp(&a.cpu_percent)
                .then(b.memory_mib.cmp(&a.memory_mib))
        });
        Ok(list)
    })
    .await
}

fn context() -> (Context, Option<u32>) {
    let probe = WindowsProbe::new();
    let display_hz = probe
        .displays()
        .ok()
        .and_then(|d| d.into_iter().find(|d| d.primary).map(|d| d.current_hz));
    let config_findings = run_audit(&probe, CivilDate::today())
        .findings
        .into_iter()
        .filter(|f| f.impact > Impact::Info)
        .collect();
    (
        Context {
            display_hz,
            config_findings,
        },
        display_hz,
    )
}

/// Починає запис гри у фоні. Результат прийде подією `diag-done`.
#[tauri::command]
pub fn diag_start(
    app: AppHandle,
    state: State<'_, AppState>,
    exe: String,
    seconds: u32,
) -> CmdResult<()> {
    let target = Target::ProcessName(exe);
    target.validate()?;
    let mut slot = lock(&state.recording);
    if slot.is_some() {
        return Err(CmdError::Failed("запис уже йде".into()));
    }
    let cancel = CancelToken::new();
    *slot = Some(cancel.clone());
    drop(slot);
    let total = seconds.clamp(10, 300);

    let (start_mode, sound) = {
        let st = lock(&state.settings);
        (st.record_start, st.record_sound)
    };

    std::thread::Builder::new()
        .name("optimax-diagnosis".into())
        .spawn(move || {
            let result = (|| -> CmdResult<DiagnosisResult> {
                let state = app.state::<AppState>();
                let pm = state.presentmon()?;
                let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
                let options = RecordOptions::new(target, Duration::from_secs(u64::from(total)))
                    .with_start(start_mode, sound);
                let rec = record_with(
                    pm,
                    &options,
                    Box::new(SysinfoSource::new()),
                    gpu,
                    &cancel,
                    &mut |p| {
                        let _ = app.emit("diag-progress", p);
                    },
                )?;
                let (ctx, display_hz) = context();
                Ok(DiagnosisResult {
                    diagnosis: diagnose(&rec, &ctx),
                    display_hz,
                    presentmon_messages: rec.presentmon_messages.clone(),
                    cancelled: rec.cancelled,
                    counted_s: rec.counted_seconds(),
                    end_reason: rec.end_reason,
                })
            })();
            *lock(&app.state::<AppState>().recording) = None;
            let _ = match result {
                Ok(r) => app.emit("diag-done", r),
                Err(e) => app.emit("diag-error", e),
            };
        })?;
    Ok(())
}

/// Зупиняє запис раніше (діагноз буде за вже записаними даними).
#[tauri::command]
pub fn diag_cancel(state: State<'_, AppState>) {
    if let Some(c) = lock(&state.recording).as_ref() {
        c.cancel();
    }
}
