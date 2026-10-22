//! Ручна перевірка аудиту: `cargo run -p optimax-audit --example audit [-- --json]`.
//! Лише читає систему, нічого не змінює.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use optimax_audit::{run_audit, SystemProbe, WindowsProbe};
    use optimax_core::{Catalog, CivilDate, Impact, Text};

    let json = std::env::args().any(|a| a == "--json");
    let probe = WindowsProbe::new();
    if json {
        // Сирі дані проби — для звірки з реальним залізом.
        println!("firmware: {:#?}", probe.firmware());
        println!("displays: {:#?}", probe.displays());
        println!("gpus: {:#?}", probe.gpus());
        println!("power: {:#?}", probe.power_plan());
        println!(
            "game_mode: {:?}, hags: {:?}",
            probe.game_mode(),
            probe.hags()
        );
        println!();
    }

    let report = run_audit(&probe, CivilDate::today());
    let c = Catalog::ukrainian()?;
    println!("== {} ==\n", c.render(&Text::new("audit.title")));
    for f in &report.findings {
        let mark = match f.impact {
            Impact::High => "[!!]",
            Impact::Moderate => "[! ]",
            Impact::Info => "[i ]",
        };
        println!(
            "{mark} {} — {}, {}",
            c.render(&f.title),
            c.render(&Text::new(f.impact.text_key())),
            c.render(&Text::new("audit.confidence").arg("v", f.confidence.percent()))
        );
        println!("     {}", c.render(&f.detail));
        if let Some(a) = &f.advice {
            println!("     → {}", c.render(a));
        }
        for e in &f.evidence {
            println!("       · {}: {}", c.render(&e.label), c.render(&e.value));
        }
        println!();
    }
    for s in &report.skipped {
        let t = Text::new("audit.skipped")
            .arg("check", c.render(&s.check.name()))
            .arg("error", &s.error);
        println!("[??] {}", c.render(&t));
    }
    if !report.has_actionable() {
        println!("{}", c.render(&Text::new("audit.nothing_to_fix")));
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Аудит системи доступний лише на Windows.");
}
