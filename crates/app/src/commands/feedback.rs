//! Звіт для відгуку (бета-тест): один JSON у `reports` захищеної теки даних, який
//! користувач переглядає й надсилає сам. Мережі немає; ім'я користувача й комп'ютера
//! замінено (див. [`crate::report`]).

use super::audit::collect_hardware;
use super::blocking;
use super::cleanup::{data_dir, journal_path};
use super::system::app_info;
use crate::dto::{AppInfo, DiagnosisResult, HardwareInfo, StartupReport};
use crate::error::{CmdError, CmdResult};
use crate::log_layer::LogLine;
use crate::report::{redact, secrets_from_env, summarize_cleanup_journal};
use crate::settings::Settings;
use crate::state::{lock, AppState};
use optimax_audit::{run_audit, AuditReport, WindowsProbe};
use optimax_cleanup::path;
use optimax_cleanup::system::{SystemTools, WinTools};
use optimax_core::CivilDate;
use serde::Serialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

/// Версія формату звіту (міняється, коли змінюється структура).
const FORMAT: u32 = 1;
const REPORTS_DIR: &str = "reports";

#[derive(Serialize)]
struct OsInfo {
    /// Напр. «Windows 11 Pro 26300».
    name: Option<String>,
    /// Збірка ядра.
    kernel: Option<String>,
}

#[derive(Serialize)]
struct FeedbackReport {
    format: u32,
    created_unix: u64,
    app: AppInfo,
    os: OsInfo,
    hardware: HardwareInfo,
    audit: AuditReport,
    settings: Settings,
    startup: StartupReport,
    last_diagnosis: Option<DiagnosisResult>,
    /// Журнал змін системи (твіки) — повністю, він невеликий.
    tweaks_journal: Value,
    /// Журнал очищення — лише підсумки сеансів.
    cleanup_journal: Value,
    /// Останні рядки журналу дій.
    log: Vec<LogLine>,
}

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

fn reports_dir() -> CmdResult<String> {
    Ok(path::join(&data_dir()?, REPORTS_DIR))
}

/// JSON-файл як значення: немає файлу — `null`, зіпсований — рядок з причиною.
fn read_json(file: &str) -> Value {
    match std::fs::read_to_string(file) {
        Ok(text) => serde_json::from_str(&text)
            .unwrap_or_else(|e| Value::String(format!("не вдалося розібрати: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(e) => Value::String(format!("не вдалося прочитати: {e}")),
    }
}

/// Збирає звіт, зберігає його й повертає шлях до файлу.
#[tauri::command]
pub async fn feedback_report(state: State<'_, AppState>) -> CmdResult<String> {
    let settings = lock(&state.settings).clone();
    let startup = lock(&state.startup).clone();
    let last_diagnosis = lock(&state.last_diagnosis).clone();
    let log: Vec<LogLine> = lock(&state.log_tail).iter().cloned().collect();
    blocking(move || {
        let created_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let report = FeedbackReport {
            format: FORMAT,
            created_unix,
            app: app_info(),
            os: OsInfo {
                name: sysinfo::System::long_os_version(),
                kernel: sysinfo::System::kernel_version(),
            },
            hardware: collect_hardware(),
            audit: run_audit(&WindowsProbe::new(), CivilDate::today()),
            settings,
            startup,
            last_diagnosis,
            tweaks_journal: read_json(&path::join(&data_dir()?, "journal.json")),
            cleanup_journal: summarize_cleanup_journal(read_json(&journal_path()?)),
            log,
        };
        let mut value = serde_json::to_value(&report).map_err(failed)?;
        redact(&mut value, &secrets_from_env());
        let text = serde_json::to_string_pretty(&value).map_err(failed)?;

        let dir = reports_dir()?;
        std::fs::create_dir_all(&dir).map_err(failed)?;
        let d = CivilDate::today();
        let name = format!(
            "optimax-report-{:04}-{:02}-{:02}-{created_unix}.json",
            d.year, d.month, d.day
        );
        let file = path::join(&dir, &name);
        std::fs::write(&file, text).map_err(failed)?;
        tracing::info!("звіт для відгуку збережено: {file}");
        Ok(file)
    })
    .await
}

/// Показує звіт у Провіднику (лише файли з теки звітів).
#[tauri::command]
pub fn feedback_reveal(path: String) -> CmdResult<()> {
    let p = path::normalize(&path).map_err(|e| CmdError::Forbidden(e.to_string()))?;
    if !path::is_within(&p, &reports_dir()?) {
        return Err(CmdError::Forbidden(format!("{p}: не звіт OptimaX")));
    }
    WinTools.reveal(&p)?;
    Ok(())
}
