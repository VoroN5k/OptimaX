//! Ручна перевірка «Щойно лагнуло» і навантаження стеження у фоні.
//!
//! `cargo run -p optimax-frametime --example watch -- <гра.exe> [секунди]` у PowerShell від імені
//! адміністратора (потрібно для PresentMon і трасування DPC). Грайте; `Ctrl+Shift+F11` зберігає
//! останні 30 с — приклад друкує, що потрапило в знімок. Раз на 10 с і в кінці — CPU (% усього
//! процесора) і пам'ять цього процесу (семплер + ETW + буфер) і PresentMon.

use optimax_frametime::watch::{WatchEvent, WatchOptions, Watcher};
use optimax_frametime::{BundledPresentMon, Target};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

const MIB: u64 = 1024 * 1024;

/// (CPU % усього процесора, пам'ять МіБ) для процесів, чиє ім'я починається з `prefix`.
fn usage(sys: &System, prefix: &str, cores: f32) -> (f32, u64) {
    sys.processes()
        .values()
        .filter(|p| {
            p.name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(prefix)
        })
        .fold((0.0, 0), |(c, m), p| {
            (c + p.cpu_usage() / cores, m + p.memory() / MIB)
        })
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(game) = args.next() else {
        anyhow::bail!("вкажіть exe гри: --example watch -- game.exe [секунди]");
    };
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(120);

    let pm = BundledPresentMon::install_default()?;
    let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
    let (watcher, events) = Watcher::start(
        &pm,
        WatchOptions::new(Target::ProcessName(game.clone())),
        Box::new(SysinfoSource::new()),
        gpu,
    )?;
    println!("Стежу за {game} {seconds} с. Ctrl+Shift+F11 у грі — зберегти останні 30 с.");

    let own = std::env::current_exe()?
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "watch".into());
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f32;
    let mut sys = System::new();
    let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    let (mut own_sum, mut pm_sum, mut n) = (0.0f32, 0.0f32, 0u32);

    let mut status = None;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut next_report = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(500)) {
            Ok(WatchEvent::Captured { recording, .. }) => {
                let dpc = recording.dpc_trace.as_ref();
                println!(
                    "знімок: {} кадрів, гри {:.1} с, знімків метрик {}, DPC подій {}, довгих {}",
                    recording.frames.len(),
                    recording.counted_seconds(),
                    recording.snapshots.len(),
                    dpc.map_or(0, |t| t.events),
                    dpc.map_or(0, |t| t.long.len()),
                );
            }
            Ok(WatchEvent::CaptureFailed { reason, .. }) => {
                println!("знімок не вдався: {reason:?}")
            }
            Ok(WatchEvent::Ended(end)) => {
                println!("стеження завершилось: {end:?}");
                break;
            }
            Ok(WatchEvent::Pressed) => println!("натиснуто — зберігаю останні 30 с…"),
            Ok(WatchEvent::Status(s)) => status = Some(s),
            Err(_) => {}
        }
        if Instant::now() >= next_report {
            next_report += Duration::from_secs(10);
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
            let (oc, om) = usage(&sys, &own, cores);
            let (pc, pmm) = usage(&sys, "presentmon", cores);
            own_sum += oc;
            pm_sum += pc;
            n += 1;
            println!("CPU: OptimaX {oc:.2} % ({om} МіБ), PresentMon {pc:.2} % ({pmm} МіБ)");
            match status {
                Some(s) => println!(
                    "буфер: гра {} · у фокусі {} · {:.0} с гри · DPC {}",
                    if s.game_running {
                        "запущена"
                    } else {
                        "не знайдена"
                    },
                    if s.in_game { "так" } else { "ні" },
                    s.buffered_s,
                    if s.dpc { "є" } else { "немає" }
                ),
                None => println!("буфер: стану ще немає (цикл стеження не відповідає?)"),
            }
        }
    }
    watcher.stop();
    if n > 0 {
        println!(
            "У середньому: OptimaX {:.2} %, PresentMon {:.2} % усього процесора",
            own_sum / n as f32,
            pm_sum / n as f32
        );
    }
    Ok(())
}
