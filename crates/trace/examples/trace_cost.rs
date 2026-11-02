//! Етап 1 «анатомії кадру»: чи знижує повне трасування FPS гри.
//!
//! `cargo run -p optimax-trace --example trace_cost -- <гра.exe> [секунди_фази]` у PowerShell від
//! імені адміністратора. Запустіть, перейдіть у гру за 10 с і стійте в одній сцені. Чотири фази
//! (за замовчуванням по 20 с): без трасування → з трасуванням → без → з. Трасування — як у
//! записі: потоки гри (з обробкою по ходу) + DxgKrnl без профайлера. FPS — з PresentMon в усіх фазах; різниця
//! «з − без» порівнюється з розкидом між однаковими фазами. Нічого в системі не змінює.

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
    use optimax_frametime::live::LiveMonitor;
    use optimax_frametime::BundledPresentMon;
    use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
    use optimax_trace::{provider_guid, ProviderSpec, Session, SessionKind, ThreadTracer};
    use std::time::{Duration, Instant};
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    /// DxgKrnl: Base (0x1) і Present (0x8000000) — без профайлера (маска з заміру на RE9).
    const DXGKRNL_KEYWORDS: u64 = 0x800_0001;

    /// Трасування як у записі: потоки гри + DxgKrnl (зупиняється при знищенні).
    struct Tracing {
        threads: ThreadTracer,
        _gpu: Option<Session>,
    }

    fn start_tracing(game_pid: u32) -> anyhow::Result<Tracing> {
        let threads = ThreadTracer::start(game_pid, qpc_now())?;
        let gpu = match provider_guid("Microsoft-Windows-DxgKrnl") {
            Some(guid) => Some(Session::start(
                "OptimaX Trace Cost GPU",
                &SessionKind::Providers(vec![ProviderSpec {
                    guid,
                    level: 4,
                    keywords: DXGKRNL_KEYWORDS,
                }]),
                |_| {},
            )?),
            None => None,
        };
        Ok(Tracing { threads, _gpu: gpu })
    }

    /// Середні FPS і 1% low за фазу.
    fn phase(live: &LiveMonitor, secs: u64) -> Option<(f64, f64)> {
        let end = Instant::now() + Duration::from_secs(secs);
        // Перші 3 с фази не рахуємо: 1% low ще пам'ятає попередню фазу.
        let count_from = Instant::now() + Duration::from_secs(3);
        let (mut fps, mut low, mut n, mut nl) = (0.0, 0.0, 0u32, 0u32);
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(500));
            let Some(s) = live.latest() else { continue };
            if Instant::now() >= count_from {
                fps += s.fps;
                n += 1;
                if let Some(l) = s.low_1pct_fps {
                    low += l;
                    nl += 1;
                }
            }
        }
        (n > 0).then(|| (fps / f64::from(n), low / f64::from(nl.max(1))))
    }

    pub fn main() -> anyhow::Result<()> {
        let mut args = std::env::args().skip(1);
        let Some(game) = args.next() else {
            anyhow::bail!("вкажіть exe гри: --example trace_cost -- re9.exe [секунди_фази]");
        };
        let secs: u64 = args
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20)
            .max(8);
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

        let pm = BundledPresentMon::install_default()?;
        let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
        let live = LiveMonitor::start(&pm, Box::new(SysinfoSource::new()), gpu)?;
        println!("Перейдіть у гру за 10 с і стійте в одній сцені. 4 фази по {secs} с.");
        std::thread::sleep(Duration::from_secs(10));

        let mut results = Vec::new();
        for (i, on) in [false, true, false, true].into_iter().enumerate() {
            let tracing = if on {
                Some(start_tracing(game_pid)?)
            } else {
                None
            };
            let r = phase(&live, secs);
            let label = if on {
                "з трасуванням"
            } else {
                "без трасування"
            };
            let kept = tracing
                .map(|t| {
                    let tr = t.threads.stop();
                    (tr.switches, tr.late, tr.lost)
                })
                .unwrap_or_default();
            match r {
                Some((f, l)) if on => println!(
                    "фаза {}: {label:<15} {f:7.1} FPS · 1% low {l:7.1} · перемикань гри {}, запізнілих {}, втрачено {}",
                    i + 1,
                    kept.0,
                    kept.1,
                    kept.2
                ),
                Some((f, l)) => println!("фаза {}: {label:<15} {f:7.1} FPS · 1% low {l:7.1}", i + 1),
                None => println!("фаза {}: {label} — гра у фокусі не малювала кадрів", i + 1),
            }
            results.push(r);
        }
        live.stop();

        let [Some(a1), Some(b1), Some(a2), Some(b2)] = results[..] else {
            println!("Не всі фази мають дані — висновку немає.");
            return Ok(());
        };
        let off = (a1.0 + a2.0) / 2.0;
        let on = (b1.0 + b2.0) / 2.0;
        let diff = (on - off) / off * 100.0;
        let low_off = (a1.1 + a2.1) / 2.0;
        let low_on = (b1.1 + b2.1) / 2.0;
        let noise = ((a1.0 - a2.0).abs() / off).max((b1.0 - b2.0).abs() / on) * 100.0;
        println!(
            "Без трасування {off:.1} FPS (1% low {low_off:.1}), з трасуванням {on:.1} FPS (1% low {low_on:.1}): {diff:+.2} % (розкид між однаковими фазами {noise:.2} %)"
        );
        if diff.abs() <= noise.max(1.0) {
            println!("Висновок: впливу трасування на FPS не видно (різниця в межах розкиду).");
        } else {
            println!("Висновок: різниця більша за розкид — повторіть замір; якщо стабільно, трасування впливає.");
        }
        Ok(())
    }
}
