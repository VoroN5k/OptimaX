//! Процес навантаження без вікна програми й без прав адміністратора: перевірка на справжньому
//! залізі. `cargo run -p optimax-stability --example gpu_worker -- <секунди> [cpu|ram]`; рядки JSON
//! у stdout, закритий stdin зупиняє тест.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);
    #[cfg(windows)]
    optimax_stability::worker::run(
        match args.get(1).map(String::as_str) {
            Some("cpu") => optimax_stability::protocol::TestKind::Cpu,
            Some("ram") => optimax_stability::protocol::TestKind::Ram,
            _ => optimax_stability::protocol::TestKind::Gpu,
        },
        seconds,
    );
    #[cfg(not(windows))]
    let _ = seconds;
}
