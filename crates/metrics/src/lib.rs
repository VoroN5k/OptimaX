//! Легкий збір системних метрик для OptimaX.
//!
//! - [`SysinfoSource`]: CPU по кожному ядру, RAM, диск, топ процесів (крейт `sysinfo`).
//! - [`NvmlGpu`]: NVIDIA GPU через NVML (завантаження, VRAM, частоти, температура, тротлінг).
//! - [`Sampler`]: окремий потік, що збирає [`Snapshot`] раз на 100–500 мс і віддає їх у канал.
//!
//! Температура CPU поки не збирається: на Windows без драйвера ядра надійного джерела немає,
//! а ACPI thermal zone не є температурою процесора.

#![forbid(unsafe_code)]

mod gpu;
mod sampler;
mod snapshot;
mod source;

pub use gpu::{open_nvml, GpuApi, NvmlGpu};
pub use sampler::{Sampler, SamplerConfig};
pub use snapshot::{
    CpuSnapshot, DiskSnapshot, GpuSnapshot, MemorySnapshot, ProcessSample, Snapshot, ThrottleReason,
};
pub use source::{SysinfoSource, SystemSource};

/// Помилки збору метрик.
#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    /// Не вдалося запустити потік збору.
    #[error("не вдалося запустити потік збору метрик: {0}")]
    Spawn(#[from] std::io::Error),
}
