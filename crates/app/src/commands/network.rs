//! Мережа: стан адаптерів і замір затримки (події `net-progress`, `net-done`, `net-error`).
//! Замір запускається лише користувачем; пінги — до роутера й до вузла, який він бачить.

use super::blocking;
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_core::Finding;
use optimax_network::{
    analyze, analyze_link, network_info, resolve, run_test, system_echo, Adapter, CancelToken,
    Measurement, NetworkInfo, PingStats, TestOptions,
};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Стан мережі для екрана.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NetView {
    /// Адаптери.
    pub info: NetworkInfo,
    /// Висновки лише за станом з'єднання (до заміру).
    pub findings: Vec<Finding>,
    /// Зовнішній вузол за замовчуванням (DNS, якщо це не роутер).
    pub default_remote: Option<String>,
}

/// Проміжна статистика заміру.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NetProgress {
    /// Минуло, с.
    pub elapsed_s: f64,
    /// Усього, с.
    pub total_s: f64,
    /// Роутер.
    pub gateway: Option<PingStats>,
    /// Зовнішній вузол.
    pub remote: Option<PingStats>,
}

/// Результат заміру.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NetResult {
    /// Виміряне.
    pub measurement: Measurement,
    /// Адаптер, через який іде інтернет.
    pub link: Option<Adapter>,
    /// Висновки: підсумок першим.
    pub findings: Vec<Finding>,
    /// Замір скасовано до кінця.
    pub cancelled: bool,
}

fn net_err(e: optimax_network::NetworkError) -> CmdError {
    CmdError::Failed(e.to_string())
}

/// Адаптери й висновки за станом з'єднання.
#[tauri::command]
pub async fn net_info() -> CmdResult<NetView> {
    blocking(|| {
        let info = network_info().map_err(net_err)?;
        Ok(NetView {
            findings: analyze_link(info.internet()),
            default_remote: info.default_remote(),
            info,
        })
    })
    .await
}

/// Починає замір: роутер + `remote` (IP або ім'я; порожнє — лише роутер).
#[tauri::command]
pub fn net_test_start(
    app: AppHandle,
    state: State<'_, AppState>,
    remote: Option<String>,
    seconds: u32,
) -> CmdResult<()> {
    let cancel = {
        let mut slot = lock(&state.net_test);
        if slot.is_some() {
            return Err(CmdError::Failed("замір мережі вже йде".into()));
        }
        let c = CancelToken::new();
        *slot = Some(c.clone());
        c
    };
    let remote = remote
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty());
    let spawned = std::thread::Builder::new()
        .name("optimax-net-test".into())
        .spawn(move || {
            let result = measure(&app, remote.as_deref(), seconds, &cancel);
            match result {
                Ok(r) => {
                    let _ = app.emit("net-done", r);
                }
                Err(e) => {
                    let _ = app.emit("net-error", e);
                }
            }
            *lock(&app.state::<AppState>().net_test) = None;
        });
    if let Err(e) = spawned {
        *lock(&state.net_test) = None;
        return Err(CmdError::Failed(e.to_string()));
    }
    Ok(())
}

fn measure(
    app: &AppHandle,
    remote: Option<&str>,
    seconds: u32,
    cancel: &CancelToken,
) -> CmdResult<NetResult> {
    let info = network_info().map_err(net_err)?;
    let link = info.internet().cloned();
    let gateway = link
        .as_ref()
        .and_then(|l| l.gateway.as_deref())
        .and_then(|g| resolve(g).ok());
    let remote_ip = remote.map(resolve).transpose().map_err(net_err)?;
    let targets: Vec<_> = gateway.iter().chain(remote_ip.iter()).copied().collect();
    if targets.is_empty() {
        return Err(CmdError::Failed(
            "немає ні шлюзу, ні вузла для заміру".into(),
        ));
    }
    tracing::info!("замір мережі {seconds} с: {targets:?}");
    let options = TestOptions::new(Duration::from_secs(u64::from(seconds)));
    let total_s = options.duration.as_secs_f64();
    let split = |stats: &[PingStats]| {
        let mut it = stats.iter().cloned();
        let g = gateway.and_then(|_| it.next());
        (g, remote_ip.and_then(|_| it.next()))
    };
    let echo = system_echo();
    let stats = run_test(
        echo.as_ref(),
        &targets,
        options,
        cancel,
        &mut |s, elapsed_s| {
            let (gateway, remote) = split(s);
            let _ = app.emit(
                "net-progress",
                NetProgress {
                    elapsed_s,
                    total_s,
                    gateway,
                    remote,
                },
            );
        },
    );
    let (gateway, remote) = split(&stats);
    let measurement = Measurement { gateway, remote };
    Ok(NetResult {
        findings: analyze(&measurement, link.as_ref()),
        measurement,
        link,
        cancelled: cancel.is_cancelled(),
    })
}

/// Зупиняє замір (результат — за виміряним часом).
#[tauri::command]
pub fn net_test_cancel(state: State<'_, AppState>) {
    if let Some(c) = lock(&state.net_test).as_ref() {
        c.cancel();
    }
}
