//! Етап 1 «анатомії кадру»: потоки гри — те саме трасування, що й у записі «Чому лагає?».
//!
//! `cargo run -p optimax-trace --example trace_threads -- <гра.exe> [секунди]` у PowerShell від
//! імені адміністратора, поки гра йде у фокусі. Друкує найзавантаженіші потоки гри: скільки
//! працювали, чекали на ядро, були заблоковані й чому, хто їх витісняв, міграції між ядрами, і
//! який потік викликає Present (рендер); плюс запізнілі й втрачені події. Нічого не змінює.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    run::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("трасування ETW доступне лише на Windows");
}

#[cfg(windows)]
mod run {
    use optimax_core::clock::qpc_now;
    use optimax_trace::threads::slot;
    use optimax_trace::ThreadTracer;
    use std::time::{Duration, Instant};
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    fn pct(part: f64, whole: f64) -> f64 {
        if whole <= 0.0 {
            0.0
        } else {
            part * 100.0 / whole
        }
    }

    pub fn main() -> anyhow::Result<()> {
        let mut args = std::env::args().skip(1);
        let Some(game) = args.next() else {
            anyhow::bail!("вкажіть exe гри: --example trace_threads -- re9.exe [секунди]");
        };
        let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let Some(game_pid) = sys
            .processes()
            .values()
            .filter(|p| p.name().to_string_lossy().eq_ignore_ascii_case(&game))
            .min_by_key(|p| p.start_time())
            .map(|p| p.pid().as_u32())
        else {
            anyhow::bail!("{game} не запущено");
        };
        println!("гра {game}: PID {game_pid}");
        let tracer = ThreadTracer::start(game_pid, qpc_now())?;
        println!("трасування {seconds} с — грайте у фокусі гри…");
        let started = Instant::now();
        std::thread::sleep(Duration::from_secs(seconds));
        let t = tracer.stop();
        let secs = started.elapsed().as_secs_f64();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let pname = |pid: u32| {
            sys.process(sysinfo::Pid::from_u32(pid)).map_or_else(
                || format!("pid {pid}"),
                |p| p.name().to_string_lossy().into_owned(),
            )
        };
        println!();
        println!(
            "потоків гри {}, перемикань за участі гри {} ({:.0}/с), запізнілих подій {}, втрачено {}",
            t.game_threads,
            t.switches,
            t.switches as f64 / secs,
            t.late,
            t.lost
        );
        let window = secs * 1000.0;
        println!(
            "{:>7} {:<18} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}  витісняв найчастіше",
            "потік", "ім'я", "працює", "ч.ядро", "синхр.", "диск", "сон", "інше", "запус/с", "мігр/с", "витісн/с"
        );
        for s in &t.threads {
            let tot = &s.totals_ms;
            let name = match (s.tid, s.presents) {
                (0, _) => "(решта потоків)".to_string(),
                (_, p) if p > 0 => format!("{} [Present]", s.name.clone().unwrap_or_default()),
                _ => s.name.clone().unwrap_or_default(),
            };
            let name: String = name.chars().take(18).collect();
            println!(
                "{:>7} {:<18} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>7.0} {:>7.0} {:>7.0}  {}",
                s.tid,
                name,
                pct(tot[slot::RUNNING], window),
                pct(tot[slot::READY], window),
                pct(tot[slot::SYNC], window),
                pct(tot[slot::PAGE_IN], window),
                pct(tot[slot::SLEEP], window),
                pct(tot[slot::OTHER], window),
                f64::from(s.runs) / secs,
                f64::from(s.migrations) / secs,
                f64::from(s.preempted) / secs,
                s.top_preemptor
                    .map(|(p, n)| format!("{} ({n})", pname(p)))
                    .unwrap_or_default()
            );
        }
        Ok(())
    }
}
