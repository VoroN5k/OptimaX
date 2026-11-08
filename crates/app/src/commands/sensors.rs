//! Розширені датчики через драйвер PawnIO: статус і показання для «Налаштувань».
//!
//! Драйвер ставить сам користувач (з pawnio.eu); програма лише бачить, що він є, і читає датчики,
//! якщо в налаштуваннях увімкнено `pawnio_sensors` (за замовчуванням вимкнено).

use super::blocking;
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_core::registry::WinRegistry;
use optimax_metrics::amd_sensors::{AmdReading, AmdSensors, SensorError};
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// Сторінка драйвера (відкривається лише за кнопкою, у браузері).
const PAWNIO_SITE: &str = "https://pawnio.eu/";

/// Стан розширених датчиків.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SensorsStatus {
    /// PawnIO не встановлено.
    NotInstalled,
    /// Встановлено, але користувач не вмикав читання.
    Disabled {
        /// Версія PawnIO.
        version: String,
    },
    /// Процесор не підтримується (поки лише AMD Ryzen).
    Unsupported {
        /// Версія PawnIO.
        version: String,
        /// Назва процесора.
        cpu: String,
    },
    /// Драйвер чи модуль відмовив.
    Failed {
        /// Версія PawnIO.
        version: String,
        /// Що сталося.
        error: String,
    },
    /// Працює.
    Ok {
        /// Версія PawnIO.
        version: String,
        /// Показання.
        reading: AmdReading,
    },
}

/// Статус і, якщо дозволено, свіже показання.
#[tauri::command]
pub async fn sensors_status(app: AppHandle) -> CmdResult<SensorsStatus> {
    blocking(move || {
        let enabled = lock(&app.state::<AppState>().settings).pawnio_sensors;
        let Some(version) = optimax_core::pawnio::installed_version(&WinRegistry) else {
            return Ok(SensorsStatus::NotInstalled);
        };
        if !enabled {
            return Ok(SensorsStatus::Disabled { version });
        }
        let opened = AmdSensors::open(&WinRegistry);
        Ok(match opened.and_then(|s| s.read()) {
            Ok(reading) => SensorsStatus::Ok { version, reading },
            Err(SensorError::Unsupported(cpu)) => SensorsStatus::Unsupported { version, cpu },
            Err(e) => {
                tracing::warn!("розширені датчики: {e}");
                SensorsStatus::Failed {
                    version,
                    error: e.to_string(),
                }
            }
        })
    })
    .await
}

/// Відкриває pawnio.eu у браузері (лише цю адресу).
#[tauri::command]
pub fn sensors_open_site() -> CmdResult<()> {
    let explorer = optimax_core::sysdirs::explorer()
        .ok_or_else(|| CmdError::Failed("не вдалося визначити теку Windows".into()))?;
    std::process::Command::new(explorer)
        .arg(PAWNIO_SITE)
        .spawn()
        .map(|_| ())
        .map_err(|e| CmdError::Failed(e.to_string()))
}
