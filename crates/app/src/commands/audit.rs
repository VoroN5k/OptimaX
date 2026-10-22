//! Аудит конфігурації та відомості про залізо (лише читання).

use super::blocking;
use crate::dto::HardwareInfo;
use crate::error::CmdResult;
use optimax_audit::{run_audit as audit, AuditReport, SystemProbe, WindowsProbe};
use optimax_core::CivilDate;

/// Запускає аудит.
#[tauri::command]
pub async fn run_audit() -> CmdResult<AuditReport> {
    blocking(|| Ok(audit(&WindowsProbe::new(), CivilDate::today()))).await
}

/// Відомості про залізо.
#[tauri::command]
pub async fn hardware_info() -> CmdResult<HardwareInfo> {
    blocking(|| {
        let probe = WindowsProbe::new();
        let sys = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::nothing().with_cpu(sysinfo::CpuRefreshKind::nothing()),
        );
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();
        Ok(HardwareInfo {
            cpu_name,
            logical_cores: u32::try_from(sys.cpus().len()).unwrap_or(0),
            physical_cores: sysinfo::System::physical_core_count()
                .and_then(|n| u32::try_from(n).ok()),
            firmware: probe.firmware().ok(),
            displays: probe.displays().unwrap_or_default(),
            gpus: probe.gpus().unwrap_or_default(),
        })
    })
    .await
}
