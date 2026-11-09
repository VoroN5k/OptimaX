//! Служба OptimaX (етап 1): стан, встановлення за згодою, перевірка зв'язку, видалення.
//! Поки служба нічого не робить, крім відповіді на `ping`; програма працює як раніше.

use super::blocking;
use crate::error::{CmdError, CmdResult};
use optimax_service::client::Client;
use optimax_service::install::{self, SvcState};
use optimax_service::protocol::Pong;
use optimax_service::{same_file, service_exe};
use serde::Serialize;

/// Що показати в «Налаштуваннях».
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ServiceView {
    /// Стан у диспетчері служб.
    pub state: SvcState,
    /// Шлях exe служби (`%ProgramFiles%\OptimaX\OptimaX.exe`).
    pub exe: Option<String>,
    /// Програма запущена саме з цього exe (інакше служба її не прийме).
    pub from_service_exe: bool,
    /// Відповідь служби, якщо зв'язок є.
    pub ping: Option<Pong>,
    /// Чому зв'язку немає.
    pub error: Option<String>,
    /// Версія служби збігається з версією програми.
    pub same_version: bool,
}

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

fn view() -> ServiceView {
    let exe = service_exe();
    let from_service_exe = std::env::current_exe()
        .ok()
        .zip(exe.as_ref())
        .is_some_and(|(me, svc)| same_file(&me, svc));
    let state = install::state().unwrap_or(SvcState::NotInstalled);
    let (ping, error) = if state != SvcState::Running {
        (None, None)
    } else if !from_service_exe {
        // Служба приймає лише свій exe — пробувати немає сенсу.
        (None, None)
    } else {
        match Client::connect().and_then(|mut c| c.ping()) {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e.to_string())),
        }
    };
    let same_version = ping
        .as_ref()
        .is_some_and(|p| p.version == env!("CARGO_PKG_VERSION"));
    ServiceView {
        state,
        exe: exe.map(|p| p.display().to_string()),
        from_service_exe,
        ping,
        error,
        same_version,
    }
}

/// Стан служби й перевірка зв'язку.
#[tauri::command]
pub async fn service_status() -> CmdResult<ServiceView> {
    blocking(|| Ok(view())).await
}

/// Встановлює службу для поточного користувача (UI показує згоду до виклику).
#[tauri::command]
pub async fn service_install() -> CmdResult<ServiceView> {
    blocking(|| {
        let sid = optimax_core::datadir::current_user_sid().map_err(failed)?;
        let exe = install::install(&sid).map_err(failed)?;
        tracing::info!("службу OptimaX встановлено: {}", exe.display());
        Ok(view())
    })
    .await
}

/// Зупиняє й видаляє службу.
#[tauri::command]
pub async fn service_uninstall() -> CmdResult<ServiceView> {
    blocking(|| {
        install::uninstall().map_err(failed)?;
        tracing::info!("службу OptimaX видалено");
        Ok(view())
    })
    .await
}
