//! Ручна перевірка збору метрик: `cargo run -p optimax-metrics --example monitor -- [секунди]`.

use optimax_metrics::{GpuApi, NvmlGpu, Sampler, SamplerConfig, SysinfoSource};
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;

fn main() -> anyhow::Result<()> {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
    if gpu.is_none() {
        println!("NVML недоступний: метрик GPU не буде");
    }
    let (sampler, rx) = Sampler::spawn(
        SamplerConfig::default(),
        Box::new(SysinfoSource::new()),
        gpu,
    )?;

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut last_print = Instant::now() - Duration::from_secs(1);
    while Instant::now() < deadline {
        let Ok(s) = rx.recv_timeout(Duration::from_secs(2)) else {
            break;
        };
        if last_print.elapsed() < Duration::from_secs(1) {
            continue;
        }
        last_print = Instant::now();
        let busiest = s
            .cpu
            .busiest_core()
            .map(|(i, p)| format!("ядро {i}: {p:.0}%"))
            .unwrap_or_default();
        let cores: Vec<String> = s
            .cpu
            .per_core_percent
            .iter()
            .map(|p| format!("{p:.0}"))
            .collect();
        println!(
            "[{:>6} мс] CPU {:.0}% (макс. {busiest}) [{}]",
            s.elapsed_ms,
            s.cpu.total_percent,
            cores.join(" ")
        );
        println!(
            "          RAM {} / {} МіБ | диск R {} КіБ/с W {} КіБ/с",
            s.memory.used_bytes / MIB,
            s.memory.total_bytes / MIB,
            s.disk.read_bytes_per_sec / 1024,
            s.disk.write_bytes_per_sec / 1024
        );
        for g in &s.gpus {
            println!(
                "          GPU{} {}: {:?}% | VRAM {:?}/{:?} МіБ | {:?} МГц | {:?} °C | {:?}/{:?} Вт | тротлінг {:?}",
                g.index,
                g.name,
                g.utilization_percent,
                g.vram_used_bytes.map(|b| b / MIB),
                g.vram_total_bytes.map(|b| b / MIB),
                g.core_clock_mhz,
                g.temperature_c,
                g.power_w,
                g.power_limit_w,
                g.throttle
            );
        }
        for p in &s.top_processes {
            println!(
                "          {:>6} {:<28} {:>5.1}% CPU {:>6} МіБ",
                p.pid,
                p.name,
                p.cpu_percent,
                p.memory_bytes / MIB
            );
        }
    }
    sampler.stop();
    Ok(())
}
