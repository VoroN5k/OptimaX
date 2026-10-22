//! Ручна перевірка режиму гри:
//!
//! ```text
//! cargo run -p optimax-gamemode --example gamemode -- check [процес.exe ...]
//! cargo run -p optimax-gamemode --example gamemode -- profile add Гра.exe [--priority above|high|unchanged] [--bg chrome.exe,Spotify.exe] [--services WSearch,SysMain] [--purge]
//! cargo run -p optimax-gamemode --example gamemode -- profile list
//! cargo run -p optimax-gamemode --example gamemode -- profile remove Гра.exe
//! cargo run -p optimax-gamemode --example gamemode -- watch      (q + Enter — вийти, усе буде повернуто)
//! cargo run -p optimax-gamemode --example gamemode -- restore    (повернути залишки після збою)
//! ```
//!
//! `check` і `profile list` нічого не змінюють. Зміни системи — лише в `watch`, і лише ті,
//! що ввімкнені в профілі, який ти сам створив і підтвердив.

#[cfg(windows)]
mod app {
    use optimax_core::memory::{MemoryApi, WinMemory};
    use optimax_core::power::WinPower;
    use optimax_core::process::{ProcessApi, WinProcesses};
    use optimax_core::registry::WinRegistry;
    use optimax_core::service::{ServiceApi, WinServices, STOPPABLE_SERVICES};
    use optimax_core::{Catalog, Text};
    use optimax_gamemode::watcher::POLL_INTERVAL;
    use optimax_gamemode::{
        end_session, restore_leftovers, scan, start_session, step, GamePriority, GameProfile,
        ProfileStore, SessionReport, WatchEvent, WatchState,
    };
    use optimax_tweaks::restore::WinRestorePoint;
    use optimax_tweaks::{Confirm, Engine, FileStore, RestorePointStatus};
    use std::io::{BufRead, Write};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};

    fn arg_value(args: &[String], name: &str) -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    }

    fn list(v: Option<String>) -> Vec<String> {
        v.map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default()
    }

    fn ask_stdin(prompt: &str) -> bool {
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        matches!(line.trim(), "y" | "Y" | "так" | "т")
    }

    /// У режимі `watch` stdin читає окремий потік, тож відповіді приходять через канал.
    struct ChannelConfirm<'a>(&'a Receiver<String>);

    impl Confirm for ChannelConfirm<'_> {
        fn proceed_without_restore_point(&self, reason: &str) -> bool {
            println!("\n⚠ Точку відновлення Windows створити не вдалося: {reason}");
            print!("Продовжити без неї (відкат усе одно можливий через журнал OptimaX)? [y/N] ");
            let _ = std::io::stdout().flush();
            matches!(
                self.0.recv().as_deref().map(str::trim),
                Ok("y" | "Y" | "так" | "т")
            )
        }
    }

    fn print_profile(c: &Catalog, p: &GameProfile) {
        println!("  Гра: {}", p.exe);
        let prio = match p.game_priority {
            GamePriority::Unchanged => "не змінювати".to_string(),
            GamePriority::AboveNormal => c.render(&Text::new("priority.above_normal")),
            GamePriority::High => c.render(&Text::new("priority.high")),
        };
        println!("  Пріоритет гри: {prio}");
        println!(
            "  Знизити пріоритет фону: {}",
            if p.lower_background.is_empty() {
                "—".into()
            } else {
                p.lower_background.join(", ")
            }
        );
        println!(
            "  Тимчасово зупинити служби: {}",
            if p.stop_services.is_empty() {
                "—".into()
            } else {
                p.stop_services.join(", ")
            }
        );
        println!(
            "  Очищати standby-пам'ять, якщо RAM мало: {}",
            if p.purge_standby_when_low {
                "так"
            } else {
                "ні"
            }
        );
    }

    fn print_report(c: &Catalog, r: &SessionReport) {
        if let Some(RestorePointStatus::Created { sequence }) = &r.restore_point {
            println!(
                "  {}",
                c.render(&Text::new("tweak.restore.created").arg("v", sequence))
            );
        }
        for t in &r.applied {
            println!("  ✓ {}", c.render(t));
        }
        for t in &r.skipped {
            println!("  · {}", c.render(t));
        }
    }

    pub fn run(args: &[String]) -> anyhow::Result<()> {
        let c = Catalog::ukrainian()?;
        let profiles = ProfileStore::default_location()
            .ok_or_else(|| anyhow::anyhow!("немає LOCALAPPDATA"))?;
        let (procs, svc, mem) = (WinProcesses, WinServices, WinMemory);

        match (
            args.first().map(String::as_str),
            args.get(1).map(String::as_str),
        ) {
            (Some("check"), _) => {
                let m = mem.status()?;
                println!(
                    "RAM: доступно {} з {} МіБ (мало: {})",
                    m.available_bytes >> 20,
                    m.total_bytes >> 20,
                    if m.is_low() { "так" } else { "ні" }
                );
                for s in STOPPABLE_SERVICES {
                    match svc.state(s) {
                        Ok(st) => println!("Служба {s}: {st:?}"),
                        Err(e) => println!("Служба {s}: {e}"),
                    }
                }
                // Додаткові імена процесів для перегляду (лише читання).
                for name in args.iter().skip(1) {
                    for f in procs.find_by_name(name)? {
                        println!("{} PID {}: {:?}", f.name, f.pid, procs.priority(&f));
                    }
                }
                for p in profiles.load()? {
                    let found = procs.find_by_name(&p.exe)?;
                    if found.is_empty() {
                        println!("{}: не запущено", p.exe);
                    }
                    for f in found {
                        println!("{} PID {}: {:?}", f.name, f.pid, procs.priority(&f));
                    }
                    for b in &p.lower_background {
                        for f in procs.find_by_name(b)? {
                            println!("  фон {} PID {}: {:?}", f.name, f.pid, procs.priority(&f));
                        }
                    }
                }
            }
            (Some("profile"), Some("add")) => {
                let exe = args
                    .get(2)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("вкажіть exe гри"))?;
                let mut p = GameProfile::new(exe);
                p.game_priority = match arg_value(args, "--priority").as_deref() {
                    None | Some("above") => GamePriority::AboveNormal,
                    Some("high") => GamePriority::High,
                    Some("unchanged") => GamePriority::Unchanged,
                    Some(o) => anyhow::bail!("невідомий пріоритет {o}"),
                };
                p.lower_background = list(arg_value(args, "--bg"));
                p.stop_services = list(arg_value(args, "--services"));
                p.purge_standby_when_low = args.iter().any(|a| a == "--purge");
                let (clean, warnings) = p.sanitized()?;
                println!("Профіль (саме це застосовуватиметься, коли гра запуститься):");
                print_profile(&c, &clean);
                for w in &warnings {
                    println!("  ⚠ {}", c.render(w));
                }
                if ask_stdin("Зберегти профіль? [y/N] ") {
                    profiles.upsert(&clean)?;
                    println!("Збережено: {}", profiles.path().display());
                }
            }
            (Some("profile"), Some("list")) => {
                for p in profiles.load()? {
                    print_profile(&c, &p);
                    println!();
                }
            }
            (Some("profile"), Some("remove")) => {
                let exe = args
                    .get(2)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("вкажіть exe гри"))?;
                println!(
                    "{}",
                    if profiles.remove(&exe)? {
                        "Видалено."
                    } else {
                        "Такого профілю немає."
                    }
                );
            }
            (Some("restore"), _) => {
                let (reg, power, restore) = (WinRegistry, WinPower, WinRestorePoint);
                let store = FileStore::default_location()
                    .ok_or_else(|| anyhow::anyhow!("немає LOCALAPPDATA"))?;
                let mut engine = Engine::new(&reg, &power, &restore, &store)?
                    .with_processes(&procs)
                    .with_services(&svc);
                engine.recover_pending()?;
                let s = restore_leftovers(&mut engine);
                println!("Повернуто: {:?}; не вдалося: {:?}", s.rolled_back, s.failed);
            }
            (Some("watch"), _) => {
                let list = profiles.load()?;
                if list.is_empty() {
                    anyhow::bail!("немає профілів — спершу `profile add`");
                }
                let (reg, power, restore) = (WinRegistry, WinPower, WinRestorePoint);
                let store = FileStore::default_location()
                    .ok_or_else(|| anyhow::anyhow!("немає LOCALAPPDATA"))?;
                let mut engine = Engine::new(&reg, &power, &restore, &store)?
                    .with_processes(&procs)
                    .with_services(&svc);
                engine.recover_pending()?;
                let left = restore_leftovers(&mut engine);
                if !left.rolled_back.is_empty() {
                    println!(
                        "Повернуто залишки попередньої сесії: {:?}",
                        left.rolled_back
                    );
                }

                let (tx, rx) = mpsc::channel::<String>();
                std::thread::spawn(move || {
                    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                });
                println!(
                    "Стежу за: {}. q + Enter — вийти.",
                    list.iter()
                        .map(|p| p.exe.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );

                let mut state = WatchState::Idle;
                let mut session: Option<SessionReport> = None;
                loop {
                    match rx.recv_timeout(POLL_INTERVAL) {
                        Ok(line) if line.trim().eq_ignore_ascii_case("q") => break,
                        Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    match step(&state, &scan(&procs, &list)) {
                        Some(WatchEvent::GameStarted { profile_exe, game }) => {
                            println!(
                                "\n▶ Запущено {} (PID {}) — вмикаю режим гри:",
                                game.name, game.pid
                            );
                            let Some(profile) = list.iter().find(|p| p.exe == profile_exe) else {
                                continue;
                            };
                            match start_session(
                                &mut engine,
                                &procs,
                                &mem,
                                profile,
                                &game,
                                &ChannelConfirm(&rx),
                            ) {
                                Ok(r) => {
                                    print_report(&c, &r);
                                    session = Some(r);
                                }
                                Err(e) => println!("  Режим гри не ввімкнено: {e}"),
                            }
                            state = WatchState::Active { profile_exe, game };
                        }
                        Some(WatchEvent::GameExited) => {
                            if let Some(r) = session.take() {
                                let s = end_session(&mut engine, &r);
                                println!(
                                    "\n■ Гру закрито — усе повернуто ({} змін).",
                                    s.rolled_back.len()
                                );
                                for (id, e) in s.failed {
                                    println!("  Не вдалося повернути #{id}: {e}");
                                }
                            }
                            state = WatchState::Idle;
                        }
                        None => {}
                    }
                }
                if let Some(r) = session.take() {
                    let s = end_session(&mut engine, &r);
                    println!("Вихід — усе повернуто ({} змін).", s.rolled_back.len());
                }
            }
            _ => anyhow::bail!("команди: check | profile add|list|remove | watch | restore"),
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
