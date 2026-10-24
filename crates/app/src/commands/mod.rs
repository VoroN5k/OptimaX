//! Tauri-команди. Довгі операції виконуються поза потоком UI (окремі потоки / spawn_blocking).

pub mod ab;
pub mod audit;
pub mod diagnosis;
pub mod gamemode;
pub mod laptop;
pub mod metrics;
pub mod network;
pub mod system;
pub mod tweaks;
pub mod upscale;

use crate::error::{CmdError, CmdResult};

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
