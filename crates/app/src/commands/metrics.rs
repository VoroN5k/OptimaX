//! Живі метрики для інтерфейсу (подія `metrics`). UI зупиняє їх, коли вікно приховане.

use crate::error::CmdResult;
use crate::state::{lock, AppState};
use optimax_metrics::{GpuApi, NvmlGpu, Sampler, SamplerConfig, SysinfoSource};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

/// Запускає збір (якщо ще не запущено). Інтервал 250–2000 мс; в економному режимі — 1000 мс.
#[tauri::command]
pub fn metrics_start(
    app: AppHandle,
    state: State<'_, AppState>,
    interval_ms: u32,
) -> CmdResult<()> {
    let mut slot = lock(&state.metrics);
    if slot.is_some() {
        return Ok(());
    }
    let economy = lock(&state.settings).economy;
    let ms = if economy {
        1000
    } else {
        interval_ms.clamp(250, 2000)
    };
    let config = SamplerConfig {
        // Семплер обмежує інтервал 100–500 мс; для UI рідше — пропускаємо зайві знімки нижче.
        interval: Duration::from_millis(u64::from(ms.min(500))),
        process_interval: Duration::from_secs(2),
        top_processes: 6,
    };
    let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
    let (sampler, rx) = Sampler::spawn(config, Box::new(SysinfoSource::new()), gpu)?;
    std::thread::Builder::new()
        .name("optimax-ui-metrics".into())
        .spawn(move || {
            let mut last = 0u64;
            // Завершується, коли семплер зупинено (канал закрито).
            for s in rx {
                if s.elapsed_ms.saturating_sub(last) + 50 < u64::from(ms) && last != 0 {
                    continue;
                }
                last = s.elapsed_ms;
                if app.emit("metrics", &s).is_err() {
                    break;
                }
            }
        })?;
    *slot = Some(sampler);
    Ok(())
}

/// Зупиняє збір.
#[tauri::command]
pub fn metrics_stop(state: State<'_, AppState>) {
    if let Some(s) = lock(&state.metrics).take() {
        s.stop();
    }
}
