//! Скан і сухий прогін на справжній системі — лише читання, нічого не видаляє.
//!
//! `cargo run --release -p optimax-cleanup --example scan -- [C:\] [full]`

use optimax_cleanup::{build_plan, scan, CancelToken, Env, RuleSet, ScanOptions, StdFs};

fn mb(b: u64) -> f64 {
    b as f64 / 1_048_576.0
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let drive = args.next().unwrap_or_else(|| r"C:\".into());
    let full = args.next().as_deref() == Some("full");
    let (rules, issues) = RuleSet::embedded(Env::from_process());
    for i in &issues {
        println!("ПРОБЛЕМА ПРАВИЛА {} ({}): {}", i.rule_id, i.file, i.problem);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    let opts = ScanOptions {
        drive,
        full,
        now_unix: now,
        threads: 4,
    };
    let report = scan(&StdFs, &rules, &opts, &CancelToken::new(), &|_| {});
    println!(
        "скан {} ({}): {} мс, неприступних тек {}",
        report.drive,
        if full {
            "повний"
        } else {
            "за правилами"
        },
        report.duration_ms,
        report.unreadable_dirs
    );
    for i in &report.items {
        println!(
            "{:>10.1} МБ  (під умови {:>9.1} МБ)  {:?}/{:?}  {}  [{}]",
            mb(i.bytes),
            mb(i.eligible_bytes),
            i.verdict,
            i.method,
            i.path,
            i.rule.as_ref().map_or("", |r| r.id.as_str())
        );
    }
    if full {
        println!(
            "--- невідоме (повний скан {:.1} ГБ):",
            mb(report.scanned_bytes) / 1024.0
        );
        for i in &report.unknown {
            println!(
                "{:>10.1} МБ  {:?} {:?}  {}",
                mb(i.bytes),
                i.verdict,
                i.zone,
                i.path
            );
        }
    }
    let selectable: Vec<String> = report
        .items
        .iter()
        .filter(|i| i.selectable)
        .map(|i| i.path.clone())
        .collect();
    let plan = build_plan(&StdFs, &rules, &selectable, now, &CancelToken::new())?;
    println!(
        "--- сухий прогін: {} файлів, видалити {:.1} МБ, у карантин {:.1} МБ, відкинуто {}, служби {:?}",
        plan.file_count(),
        mb(plan.delete_bytes()),
        mb(plan.quarantine_bytes()),
        plan.skipped.len(),
        plan.services
    );
    for s in plan.skipped.iter().take(10) {
        println!("  відкинуто: {} — {}", s.path, s.reason.describe());
    }
    Ok(())
}
