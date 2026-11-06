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
    optimax_app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("OptimaX працює лише на Windows 10/11.");
}
