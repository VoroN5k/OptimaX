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
        Some("--service") => return Ok(optimax_service::host::run()?),
        Some("--stop-service") => return Ok(optimax_service::install::stop()?),
        Some("--start-service") => return Ok(optimax_service::install::start()?),
        Some("--uninstall-service") => return Ok(optimax_service::install::uninstall()?),
        _ => {}
    }
    optimax_app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("OptimaX працює лише на Windows 10/11.");
}
