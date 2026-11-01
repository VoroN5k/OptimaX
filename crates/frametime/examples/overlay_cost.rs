//! Замір впливу оверлею на FPS гри: `cargo run -p optimax-frametime --example overlay_cost -- [секунди_фази]`
//! у PowerShell від імені адміністратора. Запустіть, перейдіть у гру й стійте в одній сцені.
//!
//! Чотири фази (за замовчуванням по 20 с): без оверлею → з оверлеєм → без → з. Живі дані
//! (PresentMon) працюють в усіх фазах, тож різниця — саме від вікна оверлею. Висновок чесний:
//! різниця «з − без» порівнюється з розкидом між однаковими фазами.

use optimax_core::overlay::{Overlay, OverlayLine};
use optimax_frametime::live::LiveMonitor;
use optimax_frametime::BundledPresentMon;
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use std::time::{Duration, Instant};

/// Середні FPS і 1% low за фазу.
fn phase(live: &LiveMonitor, overlay: Option<&Overlay>, secs: u64) -> Option<(f64, f64)> {
    let end = Instant::now() + Duration::from_secs(secs);
    // Перші 3 с фази не рахуємо: 1% low ще пам'ятає попередню фазу.
    let count_from = Instant::now() + Duration::from_secs(3);
    let (mut fps, mut low, mut n, mut nl) = (0.0, 0.0, 0u32, 0u32);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(500));
        let Some(s) = live.latest() else {
            if let Some(o) = overlay {
                o.hide();
            }
            continue;
        };
        if let Some(o) = overlay {
            o.show(vec![
                OverlayLine::normal(format!(
                    "{:.0} FPS   1% low {}",
                    s.fps,
                    s.low_1pct_fps.map_or("—".into(), |v| format!("{v:.0}"))
                )),
                OverlayLine::muted(format!("{:.1} мс", s.frametime_ms)),
                OverlayLine::muted(format!(
                    "CPU {:.0} %  ядро {:.0} %",
                    s.cpu_percent.unwrap_or(0.0),
                    s.cpu_core_percent.unwrap_or(0.0)
                )),
                OverlayLine::muted(format!(
                    "GPU {} %",
                    s.gpu_percent.map_or("—".into(), |v| v.to_string())
                )),
            ]);
        }
        if Instant::now() >= count_from {
            fps += s.fps;
            n += 1;
            if let Some(l) = s.low_1pct_fps {
                low += l;
                nl += 1;
            }
        }
    }
    if let Some(o) = overlay {
        o.hide();
    }
    (n > 0).then(|| (fps / f64::from(n), low / f64::from(nl.max(1))))
}

fn main() -> anyhow::Result<()> {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20)
        .max(8);
    let pm = BundledPresentMon::install_default()?;
    let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
    let live = LiveMonitor::start(&pm, Box::new(SysinfoSource::new()), gpu)?;
    let overlay = Overlay::new()?;
    println!("Перейдіть у гру за 10 с і стійте в одній сцені. 4 фази по {secs} с.");
    std::thread::sleep(Duration::from_secs(10));

    let mut results = Vec::new();
    for (i, on) in [false, true, false, true].into_iter().enumerate() {
        let r = phase(&live, on.then_some(&overlay), secs);
        let label = if on {
            "з оверлеєм"
        } else {
            "без оверлею"
        };
        match r {
            Some((f, l)) => println!("фаза {}: {label:<12} {f:7.1} FPS · 1% low {l:7.1}", i + 1),
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
    // Розкид: наскільки відрізняються однакові фази між собою.
    let noise = ((a1.0 - a2.0).abs() / off).max((b1.0 - b2.0).abs() / on) * 100.0;
    println!("Без оверлею {off:.1} FPS, з оверлеєм {on:.1} FPS: {diff:+.2} % (розкид між однаковими фазами {noise:.2} %)");
    if diff.abs() <= noise.max(1.0) {
        println!("Висновок: впливу оверлею на FPS не видно (різниця в межах розкиду).");
    } else {
        println!("Висновок: різниця більша за розкид — повторіть замір; якщо стабільно, оверлей впливає.");
    }
    Ok(())
}
