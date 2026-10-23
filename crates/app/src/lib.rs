//! OptimaX — вимірює, що обмежує продуктивність у іграх, і доводить ефект змін цифрами.
//!
//! Tauri-бекенд: стан, команди та події для інтерфейсу. Уся логіка — у доменних крейтах.
//! Бінарник (`main.rs`) лише викликає [`run`]; маніфест адміністратора вбудовується лише в нього.

#[cfg(windows)]
mod commands;
#[cfg(windows)]
mod dto;
#[cfg(windows)]
mod error;
#[cfg(windows)]
mod log_layer;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod state;

#[cfg(all(test, windows))]
mod bindings;

/// Запускає застосунок (блокує потік до закриття вікна).
#[cfg(windows)]
pub fn run() -> anyhow::Result<()> {
    app::run()
}

#[cfg(windows)]
mod app {
    use crate::commands::{
        ab, audit, diagnosis, gamemode, metrics, network, system, tweaks, upscale,
    };
    use crate::dto::StartupReport;
    use crate::log_layer::UiLogLayer;
    use crate::state::{lock, AppState};
    use optimax_gamemode::restore_leftovers;
    use tauri::{Emitter, Manager, RunEvent};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;

    /// Після аварійного завершення: доводить незавершені зміни й повертає залишки режиму гри.
    fn startup_recovery(state: &AppState) {
        let mut engine = lock(&state.engine);
        let mut report = StartupReport::default();
        match engine.recover_pending() {
            Ok(n) => report.recovered_pending = u32::try_from(n).unwrap_or(u32::MAX),
            Err(e) => report.error = Some(e.to_string()),
        }
        report.restored_gamemode = restore_leftovers(&mut engine).into();
        if report.recovered_pending > 0 || !report.restored_gamemode.rolled_back.is_empty() {
            tracing::warn!(
                "відновлення після збою: оброблено {} записів, повернуто змін режиму гри: {}",
                report.recovered_pending,
                report.restored_gamemode.rolled_back.len()
            );
        }
        *lock(&state.startup) = report;
    }

    /// При виході: режим гри повертає все, A/B відкочує зміну, записи й метрики зупиняються.
    fn shutdown(state: &AppState) {
        if let Some(w) = lock(&state.gamemode).take() {
            w.shutdown();
        }
        if let Some(s) = lock(&state.metrics).take() {
            s.stop();
        }
        // Апскейл зупиняється першим ділом: повертає курсор і прибирає оверлей.
        if let Some(c) = lock(&state.upscale).take() {
            c.shutdown();
        }
        if let Some(c) = lock(&state.recording).as_ref() {
            c.cancel();
        }
        if let Some(c) = lock(&state.net_test).as_ref() {
            c.cancel();
        }
        if let Err(e) = ab::cancel_session(state) {
            tracing::error!("не вдалося відкотити зміну A/B-тесту при виході: {e:?}");
        }
    }

    /// Офіційна сторінка WebView2 Runtime (Evergreen) від Microsoft.
    const WEBVIEW2_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

    /// Версія встановленого WebView2 Runtime (Evergreen) з реєстру, якщо є.
    fn webview2_version() -> Option<String> {
        use optimax_core::registry::{RegHive, RegistryApi, WinRegistry};
        const CLIENT: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
        let places = [
            (
                RegHive::LocalMachine,
                format!(r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{CLIENT}"),
            ),
            (
                RegHive::LocalMachine,
                format!(r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{CLIENT}"),
            ),
            (
                RegHive::CurrentUser,
                format!(r"Software\Microsoft\EdgeUpdate\Clients\{CLIENT}"),
            ),
        ];
        places.iter().find_map(|(hive, path)| {
            let v = WinRegistry.read_value(*hive, path, "pv").ok()??;
            let s = v.as_str()?.trim().to_string();
            (!s.is_empty() && s != "0.0.0.0").then_some(s)
        })
    }

    /// Без WebView2 веб-інтерфейс не відкриється — замість тихого падіння пояснюємо, що робити.
    /// За згодою користувача відкриваємо офіційну сторінку Microsoft; самі нічого не завантажуємо.
    fn ensure_webview2() -> bool {
        if webview2_version().is_some() {
            return true;
        }
        let catalog = optimax_core::Catalog::ukrainian().ok();
        let text = |k: &str| {
            catalog
                .as_ref()
                .map(|c| c.render(&optimax_core::Text::new(k)))
                .unwrap_or_else(|| k.to_string())
        };
        if optimax_core::dialog::ask_yes_no(&text("app.webview2.title"), &text("app.webview2.text"))
        {
            let _ = std::process::Command::new("explorer")
                .arg(WEBVIEW2_URL)
                .spawn();
        }
        false
    }

    pub fn run() -> anyhow::Result<()> {
        if !ensure_webview2() {
            return Ok(());
        }
        let (log_tx, log_rx) = std::sync::mpsc::channel();
        let _ = tracing_subscriber::registry()
            .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(UiLogLayer::new(log_tx))
            .try_init();

        let state = AppState::new()?;
        let app = tauri::Builder::default()
            .manage(state)
            .setup(move |app| {
                let handle = app.handle().clone();
                std::thread::Builder::new()
                    .name("optimax-ui-log".into())
                    .spawn(move || {
                        for line in log_rx {
                            let _ = handle.emit("log", &line);
                        }
                    })?;
                let state = app.state::<AppState>();
                startup_recovery(&state);
                let (up, hotkey) = {
                    let s = lock(&state.settings);
                    (s.upscale, s.upscale_hotkey)
                };
                match upscale::Controller::spawn(app.handle().clone(), up, hotkey) {
                    Ok(c) => *lock(&state.upscale) = Some(c),
                    Err(e) => tracing::error!("апскейл недоступний: {e:?}"),
                }
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                system::app_info,
                system::get_catalog,
                system::startup_report,
                system::get_settings,
                system::set_settings,
                audit::run_audit,
                audit::hardware_info,
                metrics::metrics_start,
                metrics::metrics_stop,
                diagnosis::list_processes,
                diagnosis::diag_start,
                diagnosis::diag_cancel,
                tweaks::power_schemes,
                tweaks::tweak_plan,
                tweaks::tweak_apply,
                tweaks::journal_entries,
                tweaks::journal_rollback,
                tweaks::journal_rollback_all,
                ab::ab_start,
                ab::ab_state,
                ab::ab_run_next,
                ab::ab_finish,
                ab::ab_cancel,
                gamemode::profiles_list,
                gamemode::profile_preview,
                gamemode::profile_save,
                gamemode::profile_remove,
                gamemode::gamemode_running,
                gamemode::gamemode_allow_without_restore_point,
                gamemode::gamemode_start,
                gamemode::gamemode_stop,
                upscale::upscale_state,
                upscale::upscale_start,
                upscale::upscale_cancel,
                upscale::upscale_stop,
                network::net_info,
                network::net_test_start,
                network::net_test_cancel,
            ])
            .build(tauri::generate_context!())?;
        app.run(|handle, event| {
            if let RunEvent::Exit = event {
                shutdown(&handle.state::<AppState>());
            }
        });
        Ok(())
    }
}
