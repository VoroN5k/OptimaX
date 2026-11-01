//! Tauri-команди. Довгі операції виконуються поза потоком UI (окремі потоки / spawn_blocking).

pub mod ab;
pub mod audit;
pub mod cleanup;
pub mod diagnosis;
pub mod feedback;
pub mod gamemode;
pub mod lag;
pub mod laptop;
pub mod metrics;
pub mod network;
pub mod resolution;
pub mod startup;
pub mod system;
pub mod tweaks;
pub mod update;
pub mod upscale;

use crate::error::{CmdError, CmdResult};
use crate::state::AppState;
use optimax_core::Catalog;
use optimax_frametime::{
    record_with, CancelToken, RecordNotices, RecordOptions, Recording, StartMode, Target,
};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Записує один прогін гри (PresentMon + метрики), хід — подією `event`. Спільне для A/B і
/// тесту роздільності. Блокує потік на весь запис.
pub(crate) fn record_game(
    app: &AppHandle,
    game: String,
    seconds: u32,
    start: StartMode,
    sound: bool,
    cancel: &CancelToken,
    event: &'static str,
) -> CmdResult<Recording> {
    let state = app.state::<AppState>();
    let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
    let mut options = RecordOptions::new(
        Target::ProcessName(game),
        Duration::from_secs(u64::from(seconds)),
    )
    .with_start(start, sound);
    if let Ok(catalog) = Catalog::ukrainian() {
        options = options.with_notices(RecordNotices::from_catalog(&catalog));
    }
    Ok(record_with(
        state.presentmon()?,
        &options,
        Box::new(SysinfoSource::new()),
        gpu,
        cancel,
        &mut |p| {
            let _ = app.emit(event, p);
        },
    )?)
}

/// Виконує блокуючу роботу в пулі потоків Tauri.
pub async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CmdError::Failed(format!("фонове завдання завершилось аварійно: {e}")))?
}
