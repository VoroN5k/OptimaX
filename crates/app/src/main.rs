//! Точка входу OptimaX.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    optimax_app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("OptimaX працює лише на Windows 10/11.");
}
