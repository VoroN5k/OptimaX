//! Ручна перевірка етапу 2 на реальній грі:
//!
//! ```text
//! cargo run --release -p optimax-diagnosis --example diagnose -- <гра.exe> [секунди] [--hz N] [--save файл.json]
//! cargo run --release -p optimax-diagnosis --example diagnose -- --load файл.json [--hz N]
//! ```
//!
//! Запускайте гру заздалегідь і грайте під час запису (не в меню). Система нічого не змінює.

use optimax_core::{Catalog, Impact, Text};
use optimax_diagnosis::{diagnose, Context};
use optimax_frametime::{
    record_with, BundledPresentMon, CancelToken, RecordNotices, RecordOptions, Recording,
    StartMode, Target,
};
use std::time::Duration;

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

#[cfg(windows)]
fn context(hz_override: Option<u32>) -> Context {
    use optimax_audit::{run_audit, SystemProbe, WindowsProbe};
    let probe = WindowsProbe::new();
    // Поки не визначаємо, на якому моніторі гра, — беремо головний (або --hz).
    let display_hz = hz_override.or_else(|| {
        probe
            .displays()
            .ok()?
            .into_iter()
            .find(|d| d.primary)
            .map(|d| d.current_hz)
    });
    let config_findings = run_audit(&probe, optimax_core::CivilDate::today()).findings;
    Context {
        display_hz,
        config_findings,
    }
}

#[cfg(not(windows))]
fn context(hz_override: Option<u32>) -> Context {
    Context {
        display_hz: hz_override,
        config_findings: Vec::new(),
    }
}

fn capture(args: &[String]) -> anyhow::Result<Recording> {
    let game = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("вкажіть exe гри, напр. GeometryDash.exe"))?;
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);

    let pm = BundledPresentMon::install_default()?;
    println!(
        "PresentMon: {} (SHA-256 перевірено; права на каталог обмежено: {})",
        pm.path().display(),
        if pm.is_dir_hardened() {
            "так"
        } else {
            "ні — запустіть від адміністратора"
        }
    );
    let gpu =
        optimax_metrics::NvmlGpu::open().map(|g| Box::new(g) as Box<dyn optimax_metrics::GpuApi>);
    println!("Запис {game} протягом {seconds} с… грайте, не згортайте гру.");
    let start = match arg_value(args, "--start").as_deref() {
        Some("auto") => StartMode::Auto,
        Some("hotkey") => StartMode::Hotkey,
        _ => StartMode::Immediate,
    };
    let options = RecordOptions::new(Target::ProcessName(game), Duration::from_secs(seconds))
        .with_start(start, true)
        .with_notices(RecordNotices::from_catalog(&Catalog::ukrainian()?));
    let mut last_phase = None;
    let rec = record_with(
        &pm,
        &options,
        Box::new(optimax_metrics::SysinfoSource::new()),
        gpu,
        &CancelToken::new(),
        &mut |p| {
            if last_phase != Some(p.phase) {
                println!(
                    "  [{:>5.1} с] фаза: {:?}, зараховано {:.1} / {:.0} с",
                    p.elapsed_s, p.phase, p.counted_s, p.target_s
                );
                last_phase = Some(p.phase);
            }
        },
    )?;
    println!(
        "Зараховано чистої гри: {:.1} с, завершення: {:?}",
        rec.counted_seconds(),
        rec.end_reason
    );
    println!(
        "Записано {} кадрів і {} знімків метрик (пропущено рядків CSV: {}).",
        rec.frames.len(),
        rec.snapshots.len(),
        rec.skipped_lines
    );
    for m in &rec.presentmon_messages {
        println!("  PresentMon: {m}");
    }
    Ok(rec)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hz = arg_value(&args, "--hz").and_then(|v| v.parse().ok());

    let rec: Recording = match arg_value(&args, "--load") {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => capture(&args)?,
    };
    if let Some(path) = arg_value(&args, "--save") {
        std::fs::write(&path, serde_json::to_string(&rec)?)?;
        println!("Запис збережено: {path}");
    }

    let ctx = context(hz);
    let d = diagnose(&rec, &ctx);
    let c = Catalog::ukrainian()?;
    println!("\n== {} ==", c.render(&Text::new("diag.title")));
    if let Some(hz) = ctx.display_hz {
        println!("(частота монітора для аналізу: {hz} Гц)");
    }
    println!();
    for v in &d.verdicts {
        let f = &v.finding;
        let mark = match f.impact {
            Impact::High => "[!!]",
            Impact::Moderate => "[! ]",
            Impact::Info => "[i ]",
        };
        println!(
            "{mark} {} — {}",
            c.render(&f.title),
            c.render(&Text::new("diag.confidence").arg("v", f.confidence.percent()))
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
    Ok(())
}
