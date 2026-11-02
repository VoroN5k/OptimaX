//! Регресійні записи для `tests/recordings.rs` (PowerShell від адміністратора, гра запущена):
//!
//! ```text
//! cargo run --release -p optimax-diagnosis --example fixture -- record <гра.exe> <секунди> <ім'я> [--desc "…"] [--hz N] [--start auto|hotkey|now]
//! cargo run --release -p optimax-diagnosis --example fixture -- import <запис.json> <ім'я> [--desc "…"] [--hz N]
//! cargo run --release -p optimax-diagnosis --example fixture -- check [ім'я]
//! ```
//!
//! `record` записує гру, знеособлює запис (імена сторонніх процесів → `app1.exe`…, без
//! повідомлень PresentMon), стискає в `tests/recordings/<ім'я>.json.gz` і пише чернетку
//! очікувань `<ім'я>.toml` з того, що показала діагностика. Чернетку **обов'язково** переглянути:
//! очікування мають описувати те, що правда для цієї сцени, а не те, що вийшло.
//! `import` — те саме з запису, збереженого `diagnose --save`. `check` — показати вердикти й
//! звірку з очікуваннями.

#[path = "../tests/fixture/mod.rs"]
mod fixture;

use optimax_core::{Catalog, Text};
use optimax_diagnosis::{diagnose, Bottleneck, Diagnosis};
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

/// Частота головного монітора (якщо не задано `--hz`).
#[cfg(windows)]
fn display_hz(over: Option<u32>) -> Option<u32> {
    use optimax_audit::{SystemProbe, WindowsProbe};
    over.or_else(|| {
        WindowsProbe::new()
            .displays()
            .ok()?
            .into_iter()
            .find(|d| d.primary)
            .map(|d| d.current_hz)
    })
}

#[cfg(not(windows))]
fn display_hz(over: Option<u32>) -> Option<u32> {
    over
}

fn capture(game: &str, seconds: u64, start: StartMode) -> anyhow::Result<Recording> {
    let pm = BundledPresentMon::install_default()?;
    let gpu =
        optimax_metrics::NvmlGpu::open().map(|g| Box::new(g) as Box<dyn optimax_metrics::GpuApi>);
    let options = RecordOptions::new(
        Target::ProcessName(game.to_string()),
        Duration::from_secs(seconds),
    )
    .with_start(start, true)
    .with_notices(RecordNotices::from_catalog(&Catalog::ukrainian()?));
    println!("Запис {game}, {seconds} с чистої гри… перейдіть у гру й грайте (не в меню).");
    let mut last = None;
    let rec = record_with(
        &pm,
        &options,
        Box::new(optimax_metrics::SysinfoSource::new()),
        gpu,
        &CancelToken::new(),
        &mut |p| {
            if last != Some(p.phase) {
                println!(
                    "  [{:>5.1} с] {:?}, зараховано {:.1} с",
                    p.elapsed_s, p.phase, p.counted_s
                );
                last = Some(p.phase);
            }
        },
    )?;
    println!(
        "Готово: {} кадрів, {} знімків метрик, {:.1} с чистої гри.",
        rec.frames.len(),
        rec.snapshots.len(),
        rec.counted_seconds()
    );
    Ok(rec)
}

fn print_diagnosis(d: &Diagnosis) -> anyhow::Result<()> {
    let c = Catalog::ukrainian()?;
    println!("Вузьке місце: {}", fixture::bottleneck_name(d));
    if let Some(s) = &d.stats {
        println!(
            "FPS {:.1}, 1% low {:.1}, 0.1% low {:.1}, кадрів {}",
            s.avg_fps, s.low_1pct_fps, s.low_01pct_fps, s.frames
        );
    }
    for v in &d.verdicts {
        let f = &v.finding;
        println!(
            "  [{}] {} — {}",
            f.id,
            c.render(&f.title),
            c.render(&Text::new("diag.confidence").arg("v", f.confidence.percent()))
        );
        if f.id.starts_with("summary.") {
            for e in f.evidence.iter().skip(4) {
                println!("      · {}: {}", c.render(&e.label), c.render(&e.value));
            }
        }
    }
    Ok(())
}

/// Чернетка очікувань із того, що показала діагностика (людина має її перевірити).
fn draft(d: &Diagnosis, description: &str, hz: Option<u32>) -> String {
    let d0 = optimax_core::CivilDate::today();
    let today = format!("{:04}-{:02}-{:02}", d0.year, d0.month, d0.day);
    let bottleneck = fixture::bottleneck_name(d);
    let conf = d
        .verdicts
        .first()
        .map_or(0, |v| v.finding.confidence.percent());
    // Запас: впевненість може гуляти між версіями правил на кілька пунктів.
    let min_conf = conf.saturating_sub(15) / 5 * 5;
    let ids: Vec<&str> = d.verdicts.iter().map(|v| v.finding.id.as_str()).collect();
    let (has, absent): (Vec<&str>, Vec<&str>) = match d.bottleneck {
        Bottleneck::Gpu => (vec!["gpu_bound"], vec!["cpu_bound"]),
        Bottleneck::Cpu => (vec!["cpu_bound"], vec!["gpu_bound"]),
        _ => (vec![], vec![]),
    };
    let has: Vec<String> = has
        .into_iter()
        .filter(|h| ids.contains(h))
        .map(|h| format!("\"{h}\""))
        .collect();
    let absent: Vec<String> = absent.into_iter().map(|a| format!("\"{a}\"")).collect();
    let fps = d.stats.as_ref().map(|s| s.avg_fps).unwrap_or(0.0);
    let hz_line = hz.map_or_else(
        || "# display_hz = 144".to_string(),
        |h| format!("display_hz = {h}"),
    );
    format!(
        "# Чернетка з `fixture record` — ПЕРЕВІР: очікування мають бути правдою для цієї сцени.\n\
         # Усі вердикти цього запису: {all}\n\
         description = \"{description}\"\n\
         recorded = \"{today}\"\n\
         {hz_line}\n\
         \n\
         [expect]\n\
         bottleneck = \"{bottleneck}\"\n\
         min_confidence = {min_conf}\n\
         has = [{has}]\n\
         absent = [{absent}]\n\
         avg_fps = [{lo:.0}.0, {hi:.0}.0]\n",
        all = ids.join(", "),
        description = description.replace('"', "'"),
        has = has.join(", "),
        absent = absent.join(", "),
        lo = (fps * 0.8).floor(),
        hi = (fps * 1.2).ceil(),
    )
}

fn save(name: &str, rec: &Recording, description: &str, hz: Option<u32>) -> anyhow::Result<()> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!("ім'я — лише латиниця, цифри, «-» і «_» (напр. re9-dlss-quality)");
    }
    let anon = rec.anonymized(fixture::KEEP_PROCESSES);
    let path = fixture::write_recording(name, &anon)?;
    let size = std::fs::metadata(&path)?.len();
    println!(
        "Запис: {} ({:.1} МБ)",
        path.display(),
        size as f64 / 1_048_576.0
    );
    let ctx = optimax_diagnosis::Context {
        display_hz: hz,
        ..Default::default()
    };
    let d = diagnose(&anon, &ctx);
    print_diagnosis(&d)?;
    let toml_path = fixture::dir().join(format!("{name}.toml"));
    if toml_path.exists() {
        println!("Очікування вже є, не перезаписую: {}", toml_path.display());
    } else {
        std::fs::write(&toml_path, draft(&d, description, hz))?;
        println!("Чернетка очікувань: {} — перевір її.", toml_path.display());
    }
    Ok(())
}

fn check(only: Option<&str>) -> anyhow::Result<()> {
    let names = fixture::names()?;
    let mut bad = 0;
    for name in names
        .iter()
        .filter(|n| only.is_none_or(|o| o == n.as_str()))
    {
        let (rec, f) = fixture::load(name)?;
        let d = diagnose(&rec, &fixture::context(&f));
        println!("\n== {name}: {} ==", f.description);
        print_diagnosis(&d)?;
        let fails = fixture::check(&d, &f.expect);
        if fails.is_empty() {
            println!("OK");
        } else {
            bad += 1;
            for x in fails {
                println!("  ✗ {x}");
            }
        }
    }
    if bad > 0 {
        anyhow::bail!("не збіглося: {bad}");
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hz_arg = arg_value(&args, "--hz").and_then(|v| v.parse().ok());
    let pos: Vec<&String> = {
        // Позиційні аргументи: усе, що не є прапорцем чи його значенням.
        let mut out = Vec::new();
        let mut skip = false;
        for a in &args {
            if skip {
                skip = false;
            } else if a.starts_with("--") {
                skip = true;
            } else {
                out.push(a);
            }
        }
        out
    };
    match pos.first().map(|s| s.as_str()) {
        Some("record") => {
            let (Some(game), Some(secs), Some(name)) = (pos.get(1), pos.get(2), pos.get(3)) else {
                anyhow::bail!("record <гра.exe> <секунди> <ім'я>");
            };
            let seconds: u64 = secs.parse()?;
            let start = match arg_value(&args, "--start").as_deref() {
                Some("hotkey") => StartMode::Hotkey,
                Some("now") => StartMode::Immediate,
                _ => StartMode::Auto,
            };
            let description =
                arg_value(&args, "--desc").unwrap_or_else(|| format!("{game}, {seconds} с"));
            let rec = capture(game, seconds, start)?;
            save(name, &rec, &description, display_hz(hz_arg))
        }
        Some("import") => {
            let (Some(file), Some(name)) = (pos.get(1), pos.get(2)) else {
                anyhow::bail!("import <запис.json> <ім'я>");
            };
            let rec: Recording = serde_json::from_str(&std::fs::read_to_string(file)?)?;
            let description = arg_value(&args, "--desc").unwrap_or_else(|| (*file).clone());
            save(name, &rec, &description, display_hz(hz_arg))
        }
        Some("check") => check(pos.get(1).map(|s| s.as_str())),
        _ => anyhow::bail!("команди: record | import | check (див. коментар у прикладі)"),
    }
}
