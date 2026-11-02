//! Етап 0 «анатомії кадру»: скільки подій дає повне трасування і скільки це коштує.
//!
//! `cargo run -p optimax-trace --example trace_probe -- <гра.exe> [секунди] [all|kernel|gpu] [маска]`
//! у PowerShell від імені адміністратора, поки гра йде у фокусі. Друкує раз на 10 с CPU цього
//! процесу й темп подій, у кінці — таблицю видів подій (з назвами з опису Windows), втрачені
//! події й обсяг даних. Четвертий аргумент — ключові слова DxgKrnl (шістнадцяткова маска, напр.
//! `0x1`; за замовчуванням усі); колонка «ключ.» у таблиці — маска кожної події. Нічого в
//! системі не змінює.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    probe::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("трасування ETW доступне лише на Windows");
}

#[cfg(windows)]
mod probe {
    use optimax_trace::{
        guid_string, provider_guid, EventKey, EventStats, ProviderSpec, Session, SessionKind,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    // Прапорці `EVENT_TRACE_FLAG_*` (значення з крейта `windows`).
    const PROCESS: u32 = 0x1;
    const THREAD: u32 = 0x2;
    const CSWITCH: u32 = 0x10;
    const DPC: u32 = 0x20;
    const INTERRUPT: u32 = 0x40;
    const DISK_IO: u32 = 0x100;
    const DISPATCHER: u32 = 0x800;
    const HARD_FAULTS: u32 = 0x2000;

    type Names = HashMap<EventKey, (String, String, String, u64)>;

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn game_pid(sys: &System, exe: &str) -> Option<u32> {
        sys.processes()
            .values()
            .filter(|p| p.name().to_string_lossy().eq_ignore_ascii_case(exe))
            .min_by_key(|p| p.start_time())
            .map(|p| p.pid().as_u32())
    }

    pub fn run() -> anyhow::Result<()> {
        let mut args = std::env::args().skip(1);
        let Some(game) = args.next() else {
            anyhow::bail!(
                "вкажіть exe гри: --example trace_probe -- re9.exe [секунди] [all|kernel|gpu]"
            );
        };
        let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
        let mode = args.next().unwrap_or_else(|| "all".into());
        let dxg_keywords = args
            .next()
            .and_then(|m| u64::from_str_radix(m.trim_start_matches("0x"), 16).ok())
            .unwrap_or(u64::MAX);
        let (kernel_on, gpu_on) = match mode.as_str() {
            "kernel" => (true, false),
            "gpu" => (false, true),
            _ => (true, true),
        };

        let mut sys = System::new();
        let kind = ProcessRefreshKind::nothing().with_cpu();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let pid = game_pid(&sys, &game);
        println!(
            "гра {game}: PID {}",
            pid.map_or("не знайдено".into(), |p| p.to_string())
        );

        let dxgkrnl = provider_guid("Microsoft-Windows-DxgKrnl");
        let dxgi = provider_guid("Microsoft-Windows-DXGI");
        println!(
            "провайдери: DxgKrnl {}, DXGI {}",
            dxgkrnl.map_or("не знайдено".into(), guid_string),
            dxgi.map_or("не знайдено".into(), guid_string)
        );
        let mut labels: HashMap<u128, &str> = HashMap::new();
        labels.insert(0x3d6fa8d1_fe05_11d0_9dda_00c04fd7ba7c, "Thread");
        labels.insert(0x3d6fa8d0_fe05_11d0_9dda_00c04fd7ba7c, "Process");
        labels.insert(0x3d6fa8d4_fe05_11d0_9dda_00c04fd7ba7c, "DiskIo");
        labels.insert(0x3d6fa8d3_fe05_11d0_9dda_00c04fd7ba7c, "PageFault");
        labels.insert(0xce1dbfb4_137e_4da6_87b0_3f59aa102cbc, "PerfInfo");
        if let Some(g) = dxgkrnl {
            labels.insert(g, "DxgKrnl");
        }
        if let Some(g) = dxgi {
            labels.insert(g, "DXGI");
        }

        let stats = Arc::new(Mutex::new(EventStats::new(pid)));
        let names: Arc<Mutex<Names>> = Arc::new(Mutex::new(HashMap::new()));
        let sink = {
            let stats = stats.clone();
            let names = names.clone();
            move |r: &optimax_trace::Record<'_>| {
                let key = r.key();
                let first = lock(&stats).add(key, r.pid(), r.data().len());
                if first {
                    // Опис події з TDH — лише для першої події кожного виду (це дорого).
                    let (task, op, ev) = r.names().unwrap_or_default();
                    lock(&names).insert(key, (task, op, ev, r.keyword()));
                }
            }
        };

        let started = Instant::now();
        let kernel = if kernel_on {
            let flags =
                PROCESS | THREAD | CSWITCH | DISPATCHER | DISK_IO | HARD_FAULTS | DPC | INTERRUPT;
            Some(Session::start(
                "OptimaX Trace Probe Kernel",
                &SessionKind::Kernel { flags },
                sink.clone(),
            )?)
        } else {
            None
        };
        let providers = match (gpu_on, dxgkrnl, dxgi) {
            (true, Some(k), Some(d)) => Some(Session::start(
                "OptimaX Trace Probe Providers",
                &SessionKind::Providers(vec![
                    ProviderSpec {
                        guid: k,
                        level: 4,
                        keywords: dxg_keywords,
                    },
                    ProviderSpec {
                        guid: d,
                        level: 4,
                        keywords: u64::MAX,
                    },
                ]),
                sink,
            )?),
            (true, ..) => {
                println!("провайдери DxgKrnl/DXGI не знайдено — лише події ядра");
                None
            }
            _ => None,
        };
        println!("трасування ({mode}) {seconds} с — грайте у фокусі гри…");

        let own = std::process::id();
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f32;
        let (mut cpu_sum, mut cpu_n) = (0.0f32, 0u32);
        let mut last_total = 0u64;
        let mut last_t = Instant::now();
        while started.elapsed() < Duration::from_secs(seconds) {
            std::thread::sleep(
                Duration::from_secs(10)
                    .min(Duration::from_secs(seconds).saturating_sub(started.elapsed())),
            );
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(own)]),
                true,
                kind,
            );
            let cpu = sys
                .process(sysinfo::Pid::from_u32(own))
                .map_or(0.0, |p| p.cpu_usage() / cores);
            cpu_sum += cpu;
            cpu_n += 1;
            let total = lock(&stats).total();
            let rate = (total - last_total) as f64 / last_t.elapsed().as_secs_f64().max(0.001);
            last_total = total;
            last_t = Instant::now();
            let lost = kernel.as_ref().map_or(0, Session::lost_so_far)
                + providers.as_ref().map_or(0, Session::lost_so_far);
            println!("CPU цього процесу {cpu:.2} % · подій {rate:.0}/с · втрачено досі {lost}");
        }
        let lost_kernel = kernel.map_or(0, Session::stop);
        let lost_providers = providers.map_or(0, Session::stop);
        let secs = started.elapsed().as_secs_f64();

        let stats = lock(&stats).clone();
        let names = lock(&names).clone();
        println!();
        println!(
            "Усього {} подій ({:.0}/с), {:.1} МБ даних ({:.2} МБ/с). Втрачено: ядро {lost_kernel}, провайдери {lost_providers}.",
            stats.total(),
            stats.total() as f64 / secs,
            stats.total_bytes() as f64 / 1_048_576.0,
            stats.total_bytes() as f64 / 1_048_576.0 / secs
        );
        if cpu_n > 0 {
            println!(
                "Середній CPU цього процесу: {:.2} % усього процесора",
                cpu_sum / cpu_n as f32
            );
        }
        println!();
        println!(
            "{:<10} {:>5} {:>4} {:>3} {:<44} {:>9} {:>8} {:>9} {:>8} {:>18}",
            "провайдер",
            "id",
            "op",
            "v",
            "назва (завдання / операція / подія)",
            "подій",
            "/с",
            "від гри",
            "КБ/с",
            "ключ."
        );
        for (k, s) in stats.sorted().into_iter().take(50) {
            let label = labels.get(&k.provider).map_or_else(
                || guid_string(k.provider)[..8].to_string(),
                |l| l.to_string(),
            );
            let (task, op, ev, keyword) = names.get(&k).cloned().unwrap_or_default();
            let name: String = [task, op, ev]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" / ")
                .chars()
                .take(44)
                .collect();
            println!(
                "{:<10} {:>5} {:>4} {:>3} {:<44} {:>9} {:>8.0} {:>9} {:>8.1} {:>#18x}",
                label,
                k.id,
                k.opcode,
                k.version,
                name,
                s.count,
                s.count as f64 / secs,
                s.of_target,
                s.bytes as f64 / 1024.0 / secs,
                keyword
            );
        }
        Ok(())
    }
}
