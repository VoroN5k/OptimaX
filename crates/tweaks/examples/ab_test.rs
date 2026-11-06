//! A/B-тест зміни на реальній грі (рушій порівнянь: порядок ABBA, 3–8 прогонів на варіант):
//!
//! ```text
//! cargo run --release -p optimax-tweaks --example ab_test -- <гра.exe> power balanced|high|<GUID> [--seconds 60] [--save звіт.md]
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
    use optimax_diagnosis::{diagnose, Bottleneck, Context, Timeline};
    use optimax_experiment::report::render_markdown;
    use optimax_experiment::{evaluate, next_variant, Outcome, Policy, Run, Status};
    use optimax_frametime::{record, BundledPresentMon, CancelToken, RecordOptions, Target};
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
        let policy = Policy::default();
        println!(
            "\nТест: прогони по {seconds} с у порядку ABBA ABBA…, від {} до {} на варіант — поки висновок не стане певним. Зміна застосовуватиметься й відкочуватиметься між прогонами.",
            policy.min_per_arm, policy.max_per_arm
        );
        if plan.info.restart_game_between_runs {
            println!("⚠ Для цієї зміни гру треба перезапускати перед кожним прогоном.");
        }
        if !ask("Почати? [y/N] ") {
            return Ok(());
        }

        let pm = BundledPresentMon::install_default()?;
        let mut active_entry: Option<u64> = None;
        let mut runs: Vec<Run> = Vec::new();
        let outcome = (|| -> anyhow::Result<()> {
            while evaluate(&runs, &policy).status == Status::Continue {
                let arm = next_variant(&runs, policy.variants);
                match (arm, active_entry) {
                    (1, None) => {
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
                    (0, Some(id)) => {
                        engine.rollback(id)?;
                        active_entry = None;
                    }
                    _ => {}
                }
                let name = if arm == 0 {
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
                    "\nПрогін {} — {name}.{restart} Підготуйте ту саму сцену й натисніть Enter…",
                    runs.len() + 1
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
                let frames = Timeline::from_recording(&rec)
                    .ok_or_else(|| anyhow::anyhow!("у прогоні немає кадрів гри"))?;
                runs.push(Run {
                    variant: arm,
                    frametimes_ms: frames.frames.iter().map(|f| f.frametime_ms).collect(),
                    gpu_temp_avg_c: (!temps.is_empty())
                        .then(|| temps.iter().sum::<f64>() / temps.len() as f64),
                    pc_latency_ms: None,
                    frame_capped: d.bottleneck == Bottleneck::FrameCap,
                    excluded: false,
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

        let report = evaluate(&runs, &policy);
        let title = c.render(&plan.info.title);
        let names = ["без зміни".to_string(), "зі зміною".to_string()];
        let md = render_markdown(
            &report,
            &c,
            &names,
            &title,
            &game,
            u32::try_from(seconds).unwrap_or(u32::MAX),
        );
        println!("\n{md}\n");
        let better = report
            .comparison(1)
            .is_some_and(|x| x.verdict == Outcome::Better);
        match (better, active_entry) {
            (true, Some(id)) => {
                if !ask("Залишити зміну? [y/N] ") {
                    engine.rollback(id)?;
                    println!("Відкочено.");
                } else {
                    println!("Зміну залишено (запис журналу #{id}).");
                }
            }
            (false, Some(id)) => {
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
