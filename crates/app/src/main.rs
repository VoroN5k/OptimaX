//! Точка входу OptimaX.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    // Процес навантаження тесту стабільності: без вікна й без решти програми.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some((kind, seconds)) = optimax_stability::protocol::parse_worker_args(&args) {
        optimax_stability::worker::run(kind, seconds);
        return Ok(());
    }
    // Служба Windows і керування нею з інсталятора (хуки NSIS, права адміністратора).
    match args.first().map(String::as_str) {
        Some("--service") => return Ok(optimax_service::host::run(optimax_backend::handler())?),
        // Агент у треї: без вікна й WebView, один на сеанс (етап 4 docs/SERVICE.md).
        Some("--agent") => return optimax_app::run_agent(),
        Some("--stop-service") => return Ok(optimax_service::install::stop()?),
        Some("--start-service") => return Ok(optimax_service::install::start()?),
        Some("--uninstall-service") => return Ok(optimax_service::install::uninstall()?),
        // Встановлення служби з програми без прав: вона запускає цей exe з UAC і чекає коду.
        Some("--install-service") => {
            let sid = args.get(1).map(String::as_str).unwrap_or_default();
            if let Err(e) = optimax_service::install::install(sid) {
                optimax_core::dialog::show_error("OptimaX", &format!("Службу не встановлено: {e}"));
                std::process::exit(1);
            }
            return Ok(());
        }
        _ => {}
    }
    optimax_app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("OptimaX працює лише на Windows 10/11.");
}
