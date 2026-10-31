//! A/B-тест зміни на реальній грі (прогони чергуються ABAB…):
//!
//! ```text
//! cargo run --release -p optimax-tweaks --example ab_test -- <гра.exe> power balanced|high|<GUID> [--runs 3] [--seconds 60] [--save звіт.md]
//! cargo run --release -p optimax-tweaks --example ab_test -- <гра.exe> game-mode on|off [...]
//! ```
//!
//! Перед кожним прогоном програма просить відтворити ту саму сцену й натиснути Enter.
//! Якщо тест перервати, зміна може лишитися застосованою — поверніть її:
//! `cargo run -p optimax-tweaks --example tweak -- rollback-all`.

#[cfg(windows)]
#[path = "tweak.rs"]
#[allow(dead_code)]
mod tweak_cli;

#[cfg(windows)]
mod app {
    use super::tweak_cli::app::{ask, desired, print_plan, Console};
    use optimax_core::power::WinPower;
    use optimax_core::registry::WinRegistry;
    use optimax_core::{Catalog, Text};
    use optimax_diagnosis::{diagnose, Bottleneck, Context};
    use optimax_frametime::{record, BundledPresentMon, CancelToken, RecordOptions, Target};
    use optimax_tweaks::ab::{
        evaluate, render_markdown, schedule, Arm, Decision, RunConditions, RunResult,
    };
    use optimax_tweaks::restore::WinRestorePoint;
    use optimax_tweaks::{Engine, FileStore, RestorePointStatus};
    use std::io::BufRead;
    use std::time::Duration;

    fn arg_value(args: &[String], name: &str) -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    }

    fn wait_enter(prompt: &str) {
        println!("{prompt}");
        let mut s = String::new();
        let _ = std::io::stdin().lock().read_line(&mut s);
    }

    pub fn run(args: &[String]) -> anyhow::Result<()> {
        let c = Catalog::ukrainian()?;
        let game = args
            .first()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("вкажіть exe гри"))?;
        let want = desired(&args[1..])?;
        let runs_per_arm: usize = arg_value(args, "--runs")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let seconds: u64 = arg_value(args, "--seconds")
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let (reg, power, restore) = (WinRegistry, WinPower, WinRestorePoint);
        let store = FileStore::default_location()?;
        let mut engine = Engine::new(&reg, &power, &restore, &store)?;
        engine.recover_pending()?;

        let plan = engine.plan(&want)?;
        print_plan(&c, &plan);
        if plan.is_noop() {
            println!("Система вже у стані B — порівнювати нічого. Спершу поверніть інше значення.");
            return Ok(());
        }
        let order = schedule(runs_per_arm);
        println!(
            "\nТест: {} прогонів по {seconds} с, порядок {}. Зміна застосовуватиметься й відкочуватиметься між прогонами.",
            order.len(),
            order.iter().map(|a| if *a == Arm::A { 'A' } else { 'B' }).collect::<String>()
        );
        if plan.info.restart_game_between_runs {
            println!("⚠ Для цієї зміни гру треба перезапускати перед кожним прогоном.");
        }
        if !ask("Почати? [y/N] ") {
            return Ok(());
        }

        let pm = BundledPresentMon::install_default()?;
        let mut active_entry: Option<u64> = None;
        let mut results = Vec::new();
        let outcome = (|| -> anyhow::Result<()> {
            for (i, arm) in order.iter().enumerate() {
                match (arm, active_entry) {
                    (Arm::B, None) => {
                        let p = engine.plan(&want)?;
                        let out = engine.apply(&p, &Console)?;
                        if let Some(RestorePointStatus::Created { sequence }) = out.restore_point {
                            println!(
                                "{}",
                                c.render(&Text::new("tweak.restore.created").arg("v", sequence))
                            );
                        }
                        active_entry = out.entry_ids.last().copied();
                    }
                    (Arm::A, Some(id)) => {
                        engine.rollback(id)?;
                        active_entry = None;
                    }
                    _ => {}
                }
                let name = if *arm == Arm::A {
                    "A (без зміни)"
                } else {
                    "B (зі зміною)"
                };
                let restart = if plan.info.restart_game_between_runs {
                    " Перезапустіть гру."
                } else {
                    ""
                };
                wait_enter(&format!(
                    "\nПрогін {}/{} — {name}.{restart} Підготуйте ту саму сцену й натисніть Enter…",
                    i + 1,
                    order.len()
                ));
                let gpu = optimax_metrics::NvmlGpu::open()
                    .map(|g| Box::new(g) as Box<dyn optimax_metrics::GpuApi>);
                let rec = record(
                    &pm,
                    &RecordOptions::new(
                        Target::ProcessName(game.clone()),
                        Duration::from_secs(seconds),
                    ),
                    Box::new(optimax_metrics::SysinfoSource::new()),
                    gpu,
                    &CancelToken::new(),
                )?;
                let d = diagnose(&rec, &Context::default());
                let stats = d
                    .stats
                    .ok_or_else(|| anyhow::anyhow!("у прогоні немає кадрів гри"))?;
                let temps: Vec<f64> = rec
                    .snapshots
                    .iter()
                    .filter_map(|s| s.gpus.iter().filter_map(|g| g.temperature_c).max())
                    .map(f64::from)
                    .collect();
                println!(
                    "  → {:.1} FPS, 1% low {:.1}{}",
                    stats.avg_fps,
                    stats.low_1pct_fps,
                    if d.bottleneck == Bottleneck::FrameCap {
                        " (ліміт FPS!)"
                    } else {
                        ""
                    }
                );
                results.push(RunResult {
                    arm: *arm,
                    avg_fps: stats.avg_fps,
                    low_1pct_fps: stats.low_1pct_fps,
                    conditions: RunConditions {
                        duration_s: stats.duration_ms / 1000.0,
                        frames: stats.frames,
                        gpu_temp_avg_c: (!temps.is_empty())
                            .then(|| temps.iter().sum::<f64>() / temps.len() as f64),
                        frame_capped: d.bottleneck == Bottleneck::FrameCap,
                    },
                });
            }
            Ok(())
        })();
        if let Err(e) = outcome {
            if let Some(id) = active_entry {
                let _ = engine.rollback(id);
                println!("Тест перервано — зміну відкочено.");
            }
            return Err(e);
        }

        let report = evaluate(&results)?;
        let title = c.render(&plan.info.title);
        let md = render_markdown(&report, &c, &title, &game);
        println!("\n{md}\n");
        match (report.decision, active_entry) {
            (Decision::Keep, Some(id)) => {
                if !ask("Залишити зміну? [y/N] ") {
                    engine.rollback(id)?;
                    println!("Відкочено.");
                } else {
                    println!("Зміну залишено (запис журналу #{id}).");
                }
            }
            (Decision::Revert, Some(id)) => {
                engine.rollback(id)?;
                println!("Зміну відкочено: приросту не підтверджено.");
            }
            _ => {}
        }
        if let Some(path) = arg_value(args, "--save") {
            std::fs::write(&path, md)?;
            println!("Звіт збережено: {path}");
        }
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    app::run(&args)
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Лише для Windows.");
}
