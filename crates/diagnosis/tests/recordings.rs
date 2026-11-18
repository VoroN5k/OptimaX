//! Регресійні тести на реальних записах ігор: діагностика кожного запису з
//! `tests/recordings/` має збігатися з очікуваннями з його `.toml`. Захищає від тихих змін
//! вердиктів після правок правил. Новий запис — приклад `fixture` (див. README у теці).

mod fixture;

use optimax_diagnosis::diagnose;

#[test]
fn recordings_match_expectations() -> anyhow::Result<()> {
    let names = fixture::names()?;
    let mut failed = Vec::new();
    for name in &names {
        let (rec, f) = fixture::load(name)?;
        let d = diagnose(&rec, &fixture::context(&f));
        let fails = fixture::check(&d, &f.expect);
        if !fails.is_empty() {
            failed.push(format!(
                "{name} ({}):\n  {}",
                f.description,
                fails.join("\n  ")
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "діагностика змінилась на {} з {} записів:\n{}",
        failed.len(),
        names.len(),
        failed.join("\n")
    );
    Ok(())
}

#[test]
fn every_recording_has_both_files() -> anyhow::Result<()> {
    let dir = fixture::dir();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(name) = file.strip_suffix(".json.gz") {
            assert!(
                dir.join(format!("{name}.toml")).is_file(),
                "{file}: немає очікувань {name}.toml"
            );
        }
        if let Some(name) = file.strip_suffix(".toml") {
            assert!(
                dir.join(format!("{name}.json.gz")).is_file(),
                "{file}: немає запису {name}.json.gz"
            );
        }
    }
    Ok(())
}

/// Сам механізм: стиснений запис читається назад без втрат, звірка ловить розбіжність.
#[test]
fn harness_roundtrip_and_check() -> anyhow::Result<()> {
    use optimax_frametime::{EndReason, Recording, Target};
    let rec = Recording {
        target: Target::ProcessName("game.exe".into()),
        qpc_frequency: 10_000_000,
        start_qpc: 0,
        frames: vec![],
        snapshots: vec![],
        skipped_lines: 0,
        presentmon_messages: vec![],
        cancelled: false,
        segments: vec![],
        end_reason: EndReason::Completed,
        start_unix_ms: None,
        dpc_trace: None,
        threads: None,
    };
    let path = std::env::temp_dir().join(format!("optimax-fixture-{}.json.gz", std::process::id()));
    fixture::write_recording_at(&path, &rec)?;
    let back = fixture::read_recording(&path)?;
    let _ = std::fs::remove_file(&path);
    assert_eq!(back.target, rec.target);

    let d = diagnose(&back, &optimax_diagnosis::Context::default());
    let ok = fixture::Expect {
        bottleneck: Some("insufficient_data".into()),
        outcome: Some("insufficient".into()),
        ..Default::default()
    };
    assert!(fixture::check(&d, &ok).is_empty());
    let wrong = fixture::Expect {
        bottleneck: Some("gpu".into()),
        has: vec!["gpu_bound".into()],
        avg_fps: Some([100.0, 200.0]),
        outcome: Some("fine".into()),
        ..Default::default()
    };
    let fails = fixture::check(&d, &wrong);
    // Вузьке місце, відсутній вердикт, немає статистики, підсумок розповіді + рядок з усіма
    // вердиктами.
    assert_eq!(fails.len(), 5, "{fails:?}");
    Ok(())
}
