//! «Чому лагає?»: запис гри (PresentMon + метрики) і діагноз.
//! Події: `diag-progress` (щосекунди), `diag-done` (результат), `diag-error`.

use super::blocking;
use crate::dto::{DiagnosisResult, DiagnosisState, ProcessEntry};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_audit::{run_audit, SystemProbe, WindowsProbe};
use optimax_core::{Catalog, CivilDate, Impact};
use optimax_diagnosis::{diagnose, CacheFile, Context};
use optimax_frametime::{record_with, CancelToken, RecordNotices, RecordOptions, Target};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Процеси для вибору гри: унікальні імена, найважчі за CPU зверху.
#[tauri::command]
pub async fn list_processes(app: AppHandle) -> CmdResult<Vec<ProcessEntry>> {
    blocking(move || {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
        let state = app.state::<AppState>();
        let mut guard = lock(&state.processes);
        // Перший запит — два заміри з короткою паузою; далі CPU рахується за час від
        // попереднього запиту (UI оновлює список раз на кілька секунд), тож без очікування.
        let sys = match guard.as_mut() {
            Some(sys) => {
                sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
                sys
            }
            None => {
                let mut sys = System::new();
                sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
                std::thread::sleep(
                    sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(300)),
                );
                sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
                guard.insert(sys)
            }
        };
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

/// Скільки рівнів тек кешу шейдерів переглядати.
const SHADER_SCAN_DEPTH: usize = 4;
/// Найбільше записів у теках кешу шейдерів за один перегляд.
const SHADER_SCAN_LIMIT: usize = 100_000;

/// Файли кешу шейдерів драйвера, змінені після `since_unix_ms`. Теки — з перевірених правил
/// очищення (категорія «Шейдер-кеші»), тож шляхи не вигадуються. Лише метадані (без вмісту),
/// посилання не розкриваються, обсяг перегляду обмежено.
pub(crate) fn shader_cache_files(since_unix_ms: i64) -> Vec<CacheFile> {
    use optimax_cleanup::rules::Category as CleanupCategory;
    use optimax_cleanup::scan::expand_roots;
    use optimax_cleanup::{Env, RuleSet, StdFs};
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    let (rules, _) = RuleSet::embedded(Env::from_system());
    let mut stack: Vec<(std::path::PathBuf, usize)> = rules
        .rules()
        .iter()
        .filter(|r| r.rule.category == CleanupCategory::ShaderCaches)
        .flat_map(|r| r.patterns.iter().flat_map(|p| expand_roots(&StdFs, p)))
        .map(|(root, _)| (std::path::PathBuf::from(root), 0))
        .collect();
    let mut out = Vec::new();
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > SHADER_SCAN_LIMIT {
                return out;
            }
            // Метадані з даних FindNextFile — без відкриття файлу й без розкриття посилань.
            let Ok(meta) = entry.metadata() else { continue };
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                continue;
            }
            if meta.is_dir() {
                if depth < SHADER_SCAN_DEPTH {
                    stack.push((entry.path(), depth + 1));
                }
                continue;
            }
            let modified_unix_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_millis()).ok());
            if let Some(ms) = modified_unix_ms.filter(|ms| *ms >= since_unix_ms) {
                out.push(CacheFile {
                    modified_unix_ms: ms,
                    bytes: meta.len(),
                });
            }
        }
    }
    out
}

/// Контекст аналізу: частота монітора й знахідки аудиту конфігурації.
pub(crate) fn context() -> (Context, Option<u32>) {
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
            shader_cache_files: Vec::new(),
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
    if super::lag::is_watching(&state) {
        return Err(super::lag::busy_error());
    }
    if super::resolution::is_running(&state) {
        return Err(CmdError::Failed(
            "іде прогін тесту роздільності — дочекайся кінця або скасуй тест".into(),
        ));
    }
    // Новий діагноз — старий тест роздільності стосувався попереднього.
    super::resolution::cancel_session(&state);
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
                let mut options = RecordOptions::new(target, Duration::from_secs(u64::from(total)))
                    .with_start(start_mode, sound);
                if let Ok(catalog) = Catalog::ukrainian() {
                    options = options.with_notices(RecordNotices::from_catalog(&catalog));
                }
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
                let (mut ctx, display_hz) = context();
                if let Some(start) = rec.start_unix_ms {
                    ctx.shader_cache_files = shader_cache_files(start);
                }
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
                Ok(r) => {
                    *lock(&app.state::<AppState>().last_diagnosis) = Some(r.clone());
                    app.emit("diag-done", r)
                }
                Err(e) => app.emit("diag-error", e),
            };
        })?;
    Ok(())
}

/// Зупиняє запис раніше (діагноз буде за вже записаними даними).
/// Стан «Чому лагає?» для повернення на сторінку: чи йде запис і останній результат цього
/// запуску програми (щоб він не зникав при переході на інший екран).
#[tauri::command]
pub fn diag_state(state: State<'_, AppState>) -> DiagnosisState {
    DiagnosisState {
        recording: lock(&state.recording).is_some(),
        last: lock(&state.last_diagnosis).clone(),
    }
}

#[tauri::command]
pub fn diag_cancel(state: State<'_, AppState>) {
    if let Some(c) = lock(&state.recording).as_ref() {
        c.cancel();
    }
}
