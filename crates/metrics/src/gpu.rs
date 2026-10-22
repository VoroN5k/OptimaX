//! Метрики GPU. Зараз лише NVIDIA (NVML); AMD/Intel — окреме завдання.

use crate::snapshot::{GpuSnapshot, ThrottleReason};
use nvml_wrapper::bitmasks::device::ThrottleReasons;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;

/// Джерело метрик відеокарт.
pub trait GpuApi: Send {
    /// Знімок усіх доступних GPU. Помилки окремих полів дають `None`, а не збій.
    fn sample(&mut self) -> Vec<GpuSnapshot>;
}

/// Відкриває NVML. `None`, якщо драйвера NVIDIA немає.
///
/// На Windows бібліотека завантажується за **абсолютним** шляхом із System32 (туди її ставить
/// драйвер), щоб програма з правами адміністратора не підхопила підкинутий `nvml.dll`
/// з поточного каталогу.
pub fn open_nvml() -> Option<Nvml> {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot")?;
        let path = std::path::Path::new(&root)
            .join("System32")
            .join("nvml.dll");
        if !path.is_file() {
            tracing::debug!("nvml.dll не знайдено: {}", path.display());
            return None;
        }
        match Nvml::builder().lib_path(path.as_os_str()).init() {
            Ok(nvml) => Some(nvml),
            Err(e) => {
                tracing::warn!("NVML не ініціалізувався: {e}");
                None
            }
        }
    }
    #[cfg(not(windows))]
    {
        Nvml::init().ok()
    }
}

/// Перетворює бітову маску NVML у список причин.
pub(crate) fn map_throttle(reasons: ThrottleReasons) -> Vec<ThrottleReason> {
    const MAP: [(ThrottleReasons, ThrottleReason); 9] = [
        (ThrottleReasons::GPU_IDLE, ThrottleReason::Idle),
        (
            ThrottleReasons::APPLICATIONS_CLOCKS_SETTING,
            ThrottleReason::ApplicationClocks,
        ),
        (ThrottleReasons::SW_POWER_CAP, ThrottleReason::SwPowerCap),
        (ThrottleReasons::HW_SLOWDOWN, ThrottleReason::HwSlowdown),
        (ThrottleReasons::SYNC_BOOST, ThrottleReason::SyncBoost),
        (
            ThrottleReasons::SW_THERMAL_SLOWDOWN,
            ThrottleReason::SwThermal,
        ),
        (
            ThrottleReasons::HW_THERMAL_SLOWDOWN,
            ThrottleReason::HwThermal,
        ),
        (
            ThrottleReasons::HW_POWER_BRAKE_SLOWDOWN,
            ThrottleReason::HwPowerBrake,
        ),
        (
            ThrottleReasons::DISPLAY_CLOCK_SETTING,
            ThrottleReason::DisplayClock,
        ),
    ];
    MAP.iter()
        .filter(|(flag, _)| reasons.contains(*flag))
        .map(|(_, r)| *r)
        .collect()
}

/// NVIDIA GPU через NVML.
pub struct NvmlGpu {
    nvml: Nvml,
}

impl NvmlGpu {
    /// Обгортка над уже відкритим NVML.
    pub fn new(nvml: Nvml) -> Self {
        Self { nvml }
    }

    /// Відкриває NVML, якщо доступний.
    pub fn open() -> Option<Self> {
        open_nvml().map(Self::new)
    }
}

impl GpuApi for NvmlGpu {
    fn sample(&mut self) -> Vec<GpuSnapshot> {
        let count = match self.nvml.device_count() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("NVML device_count: {e}");
                return Vec::new();
            }
        };
        (0..count)
            .filter_map(|index| {
                let dev = self.nvml.device_by_index(index).ok()?;
                let mem = dev.memory_info().ok();
                Some(GpuSnapshot {
                    index,
                    name: dev.name().unwrap_or_default(),
                    utilization_percent: dev.utilization_rates().ok().map(|u| u.gpu),
                    vram_used_bytes: mem.as_ref().map(|m| m.used),
                    vram_total_bytes: mem.as_ref().map(|m| m.total),
                    core_clock_mhz: dev.clock_info(Clock::Graphics).ok(),
                    memory_clock_mhz: dev.clock_info(Clock::Memory).ok(),
                    temperature_c: dev.temperature(TemperatureSensor::Gpu).ok(),
                    // NVML віддає мілівати.
                    power_w: dev.power_usage().ok().map(|mw| mw as f32 / 1000.0),
                    power_limit_w: dev.enforced_power_limit().ok().map(|mw| mw as f32 / 1000.0),
                    throttle: dev
                        .current_throttle_reasons()
                        .map(map_throttle)
                        .unwrap_or_default(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_throttle_bits() {
        let r = ThrottleReasons::SW_POWER_CAP | ThrottleReasons::HW_THERMAL_SLOWDOWN;
        assert_eq!(
            map_throttle(r),
            vec![ThrottleReason::SwPowerCap, ThrottleReason::HwThermal]
        );
        assert!(map_throttle(ThrottleReasons::NONE).is_empty());
    }
}
