//! Аудит конфігурації та відомості про залізо (лише читання).

use super::blocking;
use crate::dto::HardwareInfo;
use crate::error::{CmdError, CmdResult};
use crate::link::Link;
use crate::state::{lock, AppState};
use optimax_audit::crashes::{analyze, CrashContext, CrashReport, CRASH_DAYS};
use optimax_audit::model::FabricClocks;
use optimax_audit::rules::check_fabric;
use optimax_audit::{run_audit as audit, AuditReport, SystemProbe, WindowsProbe};
use optimax_backend::sensors::SensorRead;
use optimax_core::CivilDate;
use tauri::State;

/// Аудит разом із частотами шини пам'яті, якщо користувач дозволив PawnIO (`link` — шлях до
/// датчиків; `None` — згоди немає), а процесор і таблиця SMU підтримуються. Без цього — звичайний
/// аудит.
pub fn full_audit(link: Option<&Link>) -> AuditReport {
    let mut report = audit(&WindowsProbe::new(), CivilDate::today());
    if let Some(link) = link {
        let clocks = match link.sensors().0 {
            SensorRead::Ok { reading } => reading.clocks,
            other => {
                tracing::info!("частоти шини пам'яті недоступні: {other:?}");
                None
            }
        };
        if let Some(c) = clocks {
            report.add([check_fabric(&FabricClocks {
                fclk_mhz: c.fclk_mhz,
                uclk_mhz: c.uclk_mhz,
                mclk_mhz: c.mclk_mhz,
            })]);
        }
    }
    report
}

/// Запускає аудит.
#[tauri::command]
pub async fn run_audit(state: State<'_, AppState>) -> CmdResult<AuditReport> {
    let link = super::sensors::cpu_temp_allowed(&state).then(|| state.link.clone());
    blocking(move || Ok(full_audit(link.as_deref()))).await
}

/// «Чому вилітає?»: раптові вимкнення, сині екрани й вильоти програм за [`CRASH_DAYS`] днів
/// із журналів System і Application (лише читання).
#[tauri::command]
pub async fn crash_report(state: State<'_, AppState>) -> CmdResult<CrashReport> {
    let hard_off_by_button = lock(&state.settings).power_off_by_button;
    blocking(move || {
        let events = WindowsProbe::new()
            .crash_events(CRASH_DAYS)
            .map_err(|e| CmdError::Failed(e.to_string()))?
            .unwrap_or_default();
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Ok(analyze(
            &events,
            &CrashContext {
                hard_off_by_button,
                now_unix,
                windows_dir: optimax_core::sysdirs::windows_dir()
                    .map(|w| w.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
        ))
    })
    .await
}

/// Відомості про залізо.
#[tauri::command]
pub async fn hardware_info() -> CmdResult<HardwareInfo> {
    blocking(|| Ok(collect_hardware())).await
}

/// Збирає відомості про залізо (блокуючий виклик: WMI, SMBIOS).
pub fn collect_hardware() -> HardwareInfo {
    let probe = WindowsProbe::new();
    let sys = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_cpu(sysinfo::CpuRefreshKind::nothing()),
    );
    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_default();
    HardwareInfo {
        cpu_name,
        logical_cores: u32::try_from(sys.cpus().len()).unwrap_or(0),
        physical_cores: sysinfo::System::physical_core_count().and_then(|n| u32::try_from(n).ok()),
        firmware: probe.firmware().ok(),
        displays: probe.displays().unwrap_or_default(),
        gpus: probe.gpus().unwrap_or_default(),
    }
}
