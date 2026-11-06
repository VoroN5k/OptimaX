//! Процес навантаження без вікна програми й без прав адміністратора: перевірка на справжній
//! відеокарті. `cargo run -p optimax-stability --example gpu_worker -- <секунди>`; рядки JSON
//! у stdout, закритий stdin зупиняє тест.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);
    #[cfg(windows)]
    optimax_stability::worker::run(seconds);
    #[cfg(not(windows))]
    let _ = seconds;
}
