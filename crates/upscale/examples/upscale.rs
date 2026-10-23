//! Ручна перевірка апскейлу (етап 6):
//!
//! ```text
//! cargo run --release -p optimax-upscale --example upscale -- [--filter nearest|bilinear|lanczos|fsr|nis]
//!     [--sharpness 0.5]
//!     [--fit aspect|integer|stretch] [--delay 5] [--seconds 60] [--no-cursor] [--pid N]
//!     [--hotkey] [--bench]
//! ```
//!
//! Після запуску є `--delay` секунд, щоб перейти у вікно гри (у віконному або borderless
//! режимі з роздільністю нижчою за монітор). Масштабується вікно, що буде на передньому плані.
//! З `--pid` береться головне вікно процесу одразу, без відліку.
//! З `--hotkey` апскейл вмикається й вимикається `Ctrl+Shift+F10` у грі (з повідомленнями
//! поверх гри), приклад працює `--seconds` (за замовчуванням 300).
//! З `--bench` по черзі проганяються всі фільтри (по `--seconds`, за замовчуванням 10 с) на тому
//! самому вікні й друкується таблиця виміряної вартості. Не виходьте з гри під час заміру.
//! Оверлей ховається, коли гра не в фокусі. Система нічого не змінює.

use optimax_core::toast::{Toaster, Tone, DEFAULT_DURATION};
use optimax_core::{Catalog, Text};
use optimax_upscale::{
    foreground_window, geometry::scale_factor, is_supported, main_window_of,
    settings::DEFAULT_SHARPNESS, window::enable_per_monitor_dpi, EndReason, Filter, FitMode,
    HotkeyToggle, Summary, ToggleEvent, UpscaleSettings, UpscaleStats, Upscaler, WindowInfo,
};
use std::time::{Duration, Instant};

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let filter = match arg_value(&args, "--filter").as_deref() {
        None | Some("fsr") => Filter::Fsr,
        Some("bilinear") => Filter::Bilinear,
        Some("nearest") => Filter::Nearest,
        Some("lanczos") => Filter::Lanczos,
        Some("nis") => Filter::Nis,
        Some(other) => anyhow::bail!("невідомий фільтр: {other}"),
    };
    let fit = match arg_value(&args, "--fit").as_deref() {
        None | Some("aspect") => FitMode::Aspect,
        Some("integer") => FitMode::Integer,
        Some("stretch") => FitMode::Stretch,
        Some(other) => anyhow::bail!("невідомий режим вписування: {other}"),
    };
    let delay: u64 = arg_value(&args, "--delay")
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let hotkey = args.iter().any(|a| a == "--hotkey");
    let bench = args.iter().any(|a| a == "--bench");
    let seconds: u64 = arg_value(&args, "--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(if hotkey {
            300
        } else if bench {
            10
        } else {
            60
        });
    let sharpness: f32 = arg_value(&args, "--sharpness")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SHARPNESS);
    let settings = UpscaleSettings {
        filter,
        fit,
        sharpness,
        capture_cursor: !args.iter().any(|a| a == "--no-cursor"),
    };

    enable_per_monitor_dpi();
    anyhow::ensure!(is_supported(), "Windows Graphics Capture не підтримується");
    if hotkey {
        return run_hotkey(settings, Duration::from_secs(seconds));
    }

    let window = match arg_value(&args, "--pid").and_then(|s| s.parse().ok()) {
        Some(pid) => main_window_of(pid)
            .ok_or_else(|| anyhow::anyhow!("у процесу {pid} немає видимого вікна"))?,
        None => {
            for left in (1..=delay).rev() {
                println!("Перейдіть у вікно гри… {left}");
                std::thread::sleep(Duration::from_secs(1));
            }
            foreground_window().ok_or_else(|| anyhow::anyhow!("немає вікна в фокусі"))?
        }
    };
    println!(
        "Вікно: «{}» (PID {}), клієнт {}×{}, монітор {}×{} @ ({}, {})",
        window.title,
        window.pid,
        window.client.width,
        window.client.height,
        window.monitor.width,
        window.monitor.height,
        window.monitor.x,
        window.monitor.y,
    );
    if bench {
        return run_bench(&window, settings, Duration::from_secs(seconds));
    }

    let started = Instant::now();
    let upscaler = Upscaler::start(window.id, settings)?;
    println!(
        "Старт за {:.0} мс. Фільтр {filter:?}, різкість {sharpness}, вписування {fit:?}. Зупинка через {seconds} с або коли гру закрито.",
        started.elapsed().as_secs_f64() * 1000.0
    );

    let mut prev = upscaler.stats();
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until && upscaler.is_running() {
        std::thread::sleep(Duration::from_secs(1));
        let s = upscaler.stats();
        println!(
            "захоплено {:>4}/с, показано {:>4}/с, пропущено {:>3}/с | {}×{} → {}×{} (×{:.2}) {:?}{} {}",
            s.captured - prev.captured,
            s.presented - prev.presented,
            s.skipped - prev.skipped,
            s.source.width,
            s.source.height,
            s.dest.width,
            s.dest.height,
            scale_factor(s.source, s.dest),
            s.filter,
            if s.partial {
                " (до ×2) + білінійний"
            } else {
                ""
            },
            if s.visible {
                ""
            } else {
                "[приховано: гра не в фокусі]"
            },
        );
        println!("    {}", metrics_line(&s));
        prev = s;
    }
    let reason = upscaler.stop();
    println!("Завершено: {reason:?}");
    Ok(())
}

/// Режим гарячої клавіші: Ctrl+Shift+F10 вмикає / вимикає апскейл для гри в фокусі.
fn run_hotkey(settings: UpscaleSettings, total: Duration) -> anyhow::Result<()> {
    let catalog = Catalog::ukrainian()?;
    let toaster = Toaster::new()?;
    let mut toggle = HotkeyToggle::new(settings)?;
    println!(
        "Перейдіть у гру й натисніть Ctrl+Shift+F10 (ще раз — вимкнути). Приклад працює {} с.",
        total.as_secs()
    );
    let until = Instant::now() + total;
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
        let Some(event) = toggle.poll() else {
            continue;
        };
        let (text, tone) = match &event {
            ToggleEvent::Started { window, scale } => (
                Text::new("toast.upscale.on")
                    .arg("filter", catalog.render(&settings.filter.text()))
                    .arg("scale", format!("{scale:.2}"))
                    .arg("title", &window.title),
                Tone::Info,
            ),
            ToggleEvent::Stopped(EndReason::WindowClosed) => {
                (Text::new("toast.upscale.closed"), Tone::Warn)
            }
            ToggleEvent::Stopped(EndReason::Failed(_)) => (
                Text::new("toast.upscale.failed")
                    .arg("reason", catalog.render(&Text::new("upscale.error.thread"))),
                Tone::Danger,
            ),
            ToggleEvent::Stopped(EndReason::Stopped) => {
                (Text::new("toast.upscale.off"), Tone::Info)
            }
            ToggleEvent::Failed(e) => (
                Text::new("toast.upscale.failed").arg("reason", catalog.render(&e.text())),
                Tone::Danger,
            ),
        };
        let line = catalog.render(&text);
        println!("{line}   [{event:?}]");
        toaster.show(&line, tone, DEFAULT_DURATION);
    }
    Ok(())
}

fn summary(s: Option<Summary>) -> String {
    match s {
        Some(s) => format!("{:.2} / {:.2} мс", s.median, s.p95),
        None => "—".into(),
    }
}

/// Виміри одним рядком: медіана / p95.
fn metrics_line(s: &UpscaleStats) -> String {
    let m = &s.metrics;
    format!(
        "GPU {} | обробка {} | додана затримка {} (пізніше: {}) | CPU {:.1}% ядра",
        summary(m.gpu_ms),
        summary(m.latency_ms),
        summary(m.display_ms),
        late(m.late_percent),
        m.cpu_percent,
    )
}

fn late(p: Option<f64>) -> String {
    p.map_or_else(|| "—".into(), |p| format!("{p:.1}%"))
}

/// Порівняння фільтрів на одному вікні: по `each` на фільтр, таблиця вимірів.
fn run_bench(window: &WindowInfo, base: UpscaleSettings, each: Duration) -> anyhow::Result<()> {
    let filters = [
        Filter::Nearest,
        Filter::Bilinear,
        Filter::Lanczos,
        Filter::Fsr,
        Filter::Nis,
    ];
    let mut rows = Vec::new();
    for filter in filters {
        let settings = UpscaleSettings { filter, ..base };
        let upscaler = Upscaler::start(window.id, settings)?;
        println!("{filter:?}: {} с…", each.as_secs());
        std::thread::sleep(each);
        let stats = upscaler.stats();
        let reason = upscaler.stop();
        anyhow::ensure!(reason == EndReason::Stopped, "замір перервано: {reason:?}");
        if !stats.visible || stats.metrics.gpu_ms.is_none() {
            println!("  ⚠ оверлей був прихований (гра не в фокусі) — замір неповний");
        }
        rows.push((filter, stats));
    }
    println!();
    println!(
        "{:<9} {:>7} {:>17} {:>17} {:>17} {:>9} {:>10} {:>7}",
        "фільтр",
        "кадрів",
        "GPU мед/p95",
        "обробка мед/p95",
        "затримка мед/p95",
        "пізніше",
        "пропущено",
        "CPU %"
    );
    for (filter, s) in rows {
        let m = s.metrics;
        println!(
            "{:<9} {:>7} {:>17} {:>17} {:>17} {:>9} {:>10} {:>7.1}{}",
            format!("{filter:?}"),
            m.gpu_ms.map_or(0, |g| g.count),
            summary(m.gpu_ms),
            summary(m.latency_ms),
            summary(m.display_ms),
            late(m.late_percent),
            s.skipped,
            m.cpu_percent,
            if s.partial {
                "  (NIS до ×2 + білінійний)"
            } else {
                ""
            },
        );
    }
    println!();
    for line in [
        "GPU — час масштабування кадру на відеокарті (мітки часу D3D11).",
        "Обробка — від отримання кадру гри до нашого Present (робота OptimaX).",
        "Затримка — на скільки пізніше наш кадр з'явився на екрані, ніж Windows показав би кадр гри",
        "без апскейлу (статистика показу DXGI); кратна періоду оновлення монітора.",
        "«Пізніше» — частка кадрів, що з'явилися хоч на один V-Sync пізніше.",
        "«Пропущено» — кадри гри, що вже не встигали на свій V-Sync (пропуск повертає синхронізацію).",
        "CPU — потік апскейлу за весь замір, % одного ядра.",
    ] {
        println!("{line}");
    }
    Ok(())
}
