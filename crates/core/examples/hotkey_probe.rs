//! Перевірка глобальної гарячої клавіші: `cargo run -p optimax-core --example hotkey_probe -- f11 30`
//! (або `f9`, `f10`). Друкує кожне натискання Ctrl+Shift+<клавіша> протягом N секунд.

use optimax_core::input::{FunctionKey, GlobalHotkey};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (key, name) = match args.next().as_deref() {
        Some("f9") => (FunctionKey::F9, "F9"),
        Some("f10") => (FunctionKey::F10, "F10"),
        _ => (FunctionKey::F11, "F11"),
    };
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let hk = GlobalHotkey::ctrl_shift(key)?;
    println!("Ctrl+Shift+{name} зареєстровано, слухаю {secs} с…");
    let end = Instant::now() + Duration::from_secs(secs);
    let mut n = 0;
    while Instant::now() < end {
        if hk.pressed() {
            n += 1;
            println!("натиснуто ({n})");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("готово: натискань {n}");
    Ok(())
}
