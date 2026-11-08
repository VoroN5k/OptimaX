//! Перевірка датчиків PawnIO на реальному ПК (потрібні права адміністратора й встановлений PawnIO):
//! `cargo run -p optimax-metrics --example amd_sensors`.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use optimax_core::registry::WinRegistry;
    use optimax_metrics::amd_sensors::AmdSensors;
    println!(
        "PawnIO: {:?}",
        optimax_core::pawnio::installed_version(&WinRegistry)
    );
    let s = AmdSensors::open(&WinRegistry)?;
    println!("{s:?}");
    for _ in 0..5 {
        println!("{:?}", s.read()?);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
