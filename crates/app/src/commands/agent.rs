//! Агент у треї (етап 4а `docs/SERVICE.md`): стан, автозапуск за згодою, запуск зараз.

use crate::agent;
use crate::error::{CmdError, CmdResult};
use optimax_core::registry::WinRegistry;
use serde::Serialize;
use std::time::{Duration, Instant};

/// Що показати в «Налаштуваннях».
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentView {
    /// Програма встановлена в Program Files (агент потрібен лише разом зі службою, а служба
    /// приймає лише цей exe).
    pub available: bool,
    /// Автозапуск при вході в Windows увімкнено.
    pub autostart: bool,
    /// Агент працює в цьому сеансі.
    pub running: bool,
    /// Можна запустити зараз (не працює, а програма без прав адміністратора — агент не має
    /// успадкувати права).
    pub can_start: bool,
}

fn view() -> AgentView {
    let available = crate::elevation::from_service_exe();
    let running = agent::is_running();
    AgentView {
        available,
        autostart: agent::autostart_enabled(&WinRegistry),
        running,
        can_start: available && !running && !optimax_core::process::is_elevated(),
    }
}

/// Стан агента.
#[tauri::command]
pub fn agent_status() -> AgentView {
    view()
}

/// Вмикає чи вимикає запуск агента при вході в Windows (UI показує пояснення до ввімкнення).
#[tauri::command]
pub fn agent_set_autostart(on: bool) -> CmdResult<AgentView> {
    let exe = if on {
        if !crate::elevation::from_service_exe() {
            return Err(CmdError::Forbidden(
                "агент працює лише з OptimaX, встановленої в Program Files".into(),
            ));
        }
        Some(
            optimax_service::service_exe()
                .ok_or_else(|| CmdError::Failed("тека Program Files недоступна".into()))?,
        )
    } else {
        None
    };
    agent::set_autostart(&WinRegistry, exe.as_deref())
        .map_err(|e| CmdError::Failed(e.to_string()))?;
    tracing::info!(
        "агент у треї: автозапуск при вході в Windows {}",
        if on {
            "увімкнено"
        } else {
            "вимкнено"
        }
    );
    Ok(view())
}

/// Запускає агента зараз і трохи чекає, поки він з'явиться.
#[tauri::command]
pub async fn agent_start() -> CmdResult<AgentView> {
    super::blocking(|| {
        if !view().can_start {
            return Err(CmdError::Forbidden(
                "агента зараз запустити не можна".into(),
            ));
        }
        let exe = std::env::current_exe().map_err(|e| CmdError::Failed(e.to_string()))?;
        std::process::Command::new(exe)
            .arg(agent::ARG)
            .spawn()
            .map_err(|e| CmdError::Failed(e.to_string()))?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while !agent::is_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        tracing::info!("агент у треї запущено");
        Ok(view())
    })
    .await
}
