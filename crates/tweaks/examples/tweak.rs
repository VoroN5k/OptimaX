//! Ручна перевірка змін і відкату:
//!
//! ```text
//! cargo run -p optimax-tweaks --example tweak -- status
//! cargo run -p optimax-tweaks --example tweak -- schemes
//! cargo run -p optimax-tweaks --example tweak -- apply power balanced|high|<GUID>
//! cargo run -p optimax-tweaks --example tweak -- apply game-mode on|off
//! cargo run -p optimax-tweaks --example tweak -- rollback <номер>
//! cargo run -p optimax-tweaks --example tweak -- rollback-all
//! ```
//!
//! Кожна зміна показується до застосування й вимагає підтвердження `y`.

#[cfg(windows)]
/// Консольний інтерфейс (спільний із прикладом `ab_test`).
pub mod app {
    use optimax_core::power::{guids, PowerApi, WinPower};
    use optimax_core::registry::WinRegistry;
    use optimax_core::{Catalog, Text};
    use optimax_tweaks::restore::WinRestorePoint;
    use optimax_tweaks::{
        Confirm, Desired, Engine, EntryState, FileStore, Plan, RestorePointStatus,
    };
    use std::io::{BufRead, Write};

    /// Підтвердження через консоль.
    pub struct Console;

    impl Confirm for Console {
        fn proceed_without_restore_point(&self, reason: &str) -> bool {
            let c = Catalog::ukrainian().ok();
            let text = Text::new("tweak.restore.failed").arg("reason", reason);
            println!(
                "\n⚠ {}",
                c.map(|c| c.render(&text))
                    .unwrap_or_else(|| reason.to_string())
            );
            ask("Продовжити без точки відновлення? [y/N] ")
        }
    }

    /// Питання «так/ні»; за замовчуванням — ні.
    pub fn ask(prompt: &str) -> bool {
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        matches!(line.trim(), "y" | "Y" | "так" | "т")
    }

    /// Бажаний стан з аргументів командного рядка.
    pub fn desired(args: &[String]) -> anyhow::Result<Desired> {
        match (
            args.first().map(String::as_str),
            args.get(1).map(String::as_str),
        ) {
            (Some("power"), Some("balanced")) => Ok(Desired::PowerScheme(guids::BALANCED.into())),
            (Some("power"), Some("high")) => {
                Ok(Desired::PowerScheme(guids::HIGH_PERFORMANCE.into()))
            }
            (Some("power"), Some(guid)) => Ok(Desired::PowerScheme(guid.into())),
            (Some("game-mode"), Some("on")) => Ok(Desired::GameMode(true)),
            (Some("game-mode"), Some("off")) => Ok(Desired::GameMode(false)),
            _ => anyhow::bail!("очікується: power balanced|high|<GUID> або game-mode on|off"),
        }
    }

    /// Показує «що зміниться» до застосування.
    pub fn print_plan(c: &Catalog, plan: &Plan) {
        println!("\n== {} ==", c.render(&plan.info.title));
        println!("{}", c.render(&plan.info.what));
        if plan.is_noop() {
            println!("\n{}", c.render(&Text::new("tweak.preview.noop")));
            return;
        }
        for ch in &plan.changes {
            println!(
                "\n  {}: {}",
                c.render(&Text::new("tweak.preview.where")),
                ch.target.location()
            );
            println!(
                "  {}: {}",
                c.render(&Text::new("tweak.preview.old")),
                c.render(&ch.old_display)
            );
            println!(
                "  {}: {}",
                c.render(&Text::new("tweak.preview.new")),
                c.render(&ch.new_display)
            );
        }
        println!();
        for (label, text) in [
            ("tweak.preview.performance", &plan.info.performance),
            ("tweak.preview.heat", &plan.info.heat),
            ("tweak.preview.safety", &plan.info.safety),
        ] {
            println!("  {}: {}", c.render(&Text::new(label)), c.render(text));
        }
    }

    /// Точка входу.
    pub fn run(args: &[String]) -> anyhow::Result<()> {
        let c = Catalog::ukrainian()?;
        let (reg, power, restore) = (WinRegistry, WinPower, WinRestorePoint);
        let store =
            FileStore::default_location().ok_or_else(|| anyhow::anyhow!("немає LOCALAPPDATA"))?;
        let mut engine = Engine::new(&reg, &power, &restore, &store)?;
        let recovered = engine.recover_pending()?;
        if recovered > 0 {
            println!("Після попереднього збою оброблено записів журналу: {recovered}");
        }
        match args.first().map(String::as_str) {
            Some("status") | None => {
                println!("Журнал: {}", store.path().display());
                for e in engine.entries() {
                    let state = match &e.state {
                        EntryState::Pending => "очікує".to_string(),
                        EntryState::Applied => "ДІЄ".to_string(),
                        EntryState::RolledBack { .. } => "відкочено".to_string(),
                        EntryState::Failed { error } => format!("помилка: {error}"),
                    };
                    println!(
                        "  #{:<3} {:<13} {:<60} {state}",
                        e.id,
                        e.tweak_id,
                        e.target.location()
                    );
                }
                if engine.entries().is_empty() {
                    println!("  (порожньо)");
                }
            }
            Some("schemes") => {
                let active = power.active_scheme()?;
                for s in power.schemes()? {
                    let mark = if s.guid == active { "*" } else { " " };
                    println!("{mark} {} {}", s.guid, s.name.unwrap_or_default());
                }
            }
            Some("apply") => {
                let plan = engine.plan(&desired(&args[1..])?)?;
                print_plan(&c, &plan);
                if plan.is_noop() || !ask("\nЗастосувати? [y/N] ") {
                    return Ok(());
                }
                let out = engine.apply(&plan, &Console)?;
                if let Some(RestorePointStatus::Created { sequence }) = out.restore_point {
                    println!(
                        "{}",
                        c.render(&Text::new("tweak.restore.created").arg("v", sequence))
                    );
                }
                println!(
                    "Застосовано, записи журналу: {:?}. Відкат: rollback <номер> або rollback-all",
                    out.entry_ids
                );
            }
            Some("rollback") => {
                let id: u64 = args
                    .get(1)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("вкажіть номер запису"))?;
                engine.rollback(id)?;
                println!("Відкочено #{id}");
            }
            Some("rollback-all") => {
                let s = engine.rollback_all();
                println!("Відкочено: {:?}", s.rolled_back);
                for (id, e) in s.failed {
                    println!("Не вдалося відкотити #{id}: {e}");
                }
            }
            Some(other) => anyhow::bail!("невідома команда {other}"),
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
