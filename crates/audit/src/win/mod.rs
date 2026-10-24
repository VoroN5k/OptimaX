//! Реалізація [`SystemProbe`] для Windows. Лише читання, жодних змін у системі.
#![allow(unsafe_code)]

mod battery;
mod devices;
mod display;
mod firmware;
mod hags;
mod power;

use crate::model::{
    BatteryInfo, DisplayInfo, FirmwareInfo, GameModeState, GpuAdapter, HagsInfo, PowerPlan,
};
use crate::probe::{read_game_mode, read_hags, SystemProbe};
use nvml_wrapper::Nvml;
use optimax_core::registry::WinRegistry;
use optimax_core::CoreError;

/// Проба реальної системи Windows.
pub struct WindowsProbe {
    registry: WinRegistry,
    nvml: Option<Nvml>,
}

impl WindowsProbe {
    /// Створює пробу. NVML (для Resizable BAR і VRAM на NVIDIA) відкривається, якщо доступний.
    pub fn new() -> Self {
        Self {
            registry: WinRegistry,
            nvml: optimax_metrics::open_nvml(),
        }
    }
}

impl Default for WindowsProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemProbe for WindowsProbe {
    fn firmware(&self) -> Result<FirmwareInfo, CoreError> {
        firmware::read()
    }

    fn displays(&self) -> Result<Vec<DisplayInfo>, CoreError> {
        display::read()
    }

    fn gpus(&self) -> Result<Vec<GpuAdapter>, CoreError> {
        devices::read(self.nvml.as_ref())
    }

    fn power_plan(&self) -> Result<PowerPlan, CoreError> {
        power::read()
    }

    fn game_mode(&self) -> Result<GameModeState, CoreError> {
        read_game_mode(&self.registry)
    }

    fn hags(&self) -> Result<HagsInfo, CoreError> {
        let adapters = match hags::adapters() {
            Ok(a) => Some(a),
            Err(e) => {
                tracing::warn!("стан HAGS від драйвера недоступний: {e}");
                None
            }
        };
        Ok(HagsInfo {
            registry: read_hags(&self.registry)?,
            adapters,
        })
    }

    fn battery_info(&self) -> Result<Option<BatteryInfo>, CoreError> {
        battery::read()
    }
}
