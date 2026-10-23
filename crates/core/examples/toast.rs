//! Ручна перевірка повідомлень поверх гри:
//!
//! ```text
//! cargo run -p optimax-core --example toast
//! ```
//!
//! Показує по черзі чотири тони на моніторі, де зараз активне вікно.

use optimax_core::toast::{Toaster, Tone, DEFAULT_DURATION};
use optimax_core::{Catalog, Text};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let catalog = Catalog::ukrainian()?;
    let toaster = Toaster::new()?;
    let samples = [
        (Text::new("toast.rec.warmup").arg("s", 3), Tone::Info),
        (Text::new("toast.rec.finished"), Tone::Ok),
        (
            Text::new("toast.upscale.on")
                .arg("filter", "FSR 1")
                .arg("scale", "2.00"),
            Tone::Info,
        ),
        (
            Text::new("toast.upscale.failed").arg("reason", "вікно гри згорнуте"),
            Tone::Danger,
        ),
    ];
    for (text, tone) in samples {
        let line = catalog.render(&text);
        println!("{line}");
        toaster.show(&line, tone, DEFAULT_DURATION);
        std::thread::sleep(DEFAULT_DURATION + Duration::from_millis(300));
    }
    Ok(())
}
