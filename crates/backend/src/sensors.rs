//! Датчики CPU через PawnIO з кешем відкритих модулів (без повторного завантаження на кожен замір).

use optimax_metrics::amd_sensors::AmdReading;
use serde::{Deserialize, Serialize};

/// Результат читання датчиків (так само передається каналом служби).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SensorRead {
    /// Прочитано.
    Ok {
        /// Показання.
        reading: AmdReading,
    },
    /// Процесор не підтримується.
    Unsupported {
        /// Назва процесора.
        cpu: String,
    },
    /// Драйвер чи модуль відмовив.
    Failed {
        /// Що сталося.
        error: String,
    },
}

#[cfg(windows)]
pub use self::win::SensorCache;

#[cfg(windows)]
mod win {
    use super::SensorRead;
    use optimax_core::registry::WinRegistry;
    use optimax_metrics::amd_sensors::{AmdSensors, SensorError};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Відкриті модулі PawnIO. Невдале читання закриває їх, наступне — відкриває знову
    /// (драйвер могли перевстановити чи вимкнути).
    #[derive(Debug, Default)]
    pub struct SensorCache {
        inner: Mutex<Option<AmdSensors>>,
    }

    impl SensorCache {
        /// Порожній кеш (нічого не відкриває, доки не попросять).
        pub fn new() -> Self {
            Self::default()
        }

        fn lock(&self) -> MutexGuard<'_, Option<AmdSensors>> {
            self.inner.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn with<T>(
            &self,
            f: impl FnOnce(&AmdSensors) -> Result<T, SensorError>,
        ) -> Result<T, SensorError> {
            let mut slot = self.lock();
            if slot.is_none() {
                *slot = Some(AmdSensors::open(&WinRegistry)?);
            }
            let Some(s) = slot.as_ref() else {
                return Err(SensorError::Driver("модулі не відкрито".into()));
            };
            let r = f(s);
            if r.is_err() {
                *slot = None;
            }
            r
        }

        /// Повне читання: температури й частоти.
        pub fn read(&self) -> SensorRead {
            match self.with(AmdSensors::read) {
                Ok(reading) => SensorRead::Ok { reading },
                Err(SensorError::Unsupported(cpu)) => SensorRead::Unsupported { cpu },
                Err(e) => SensorRead::Failed {
                    error: e.to_string(),
                },
            }
        }

        /// Лише температура CPU (`None` — недоступна).
        pub fn temp(&self) -> Option<f32> {
            self.with(AmdSensors::read_temp).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_result_survives_the_pipe_as_json() {
        let r = SensorRead::Ok {
            reading: AmdReading {
                cpu_c: 47.5,
                ccd_c: vec![35.0],
                clocks: None,
                pm_version: Some(0x0038_0905),
            },
        };
        let v = serde_json::to_value(&r).expect("json");
        assert_eq!(v["state"], "ok");
        assert_eq!(serde_json::from_value::<SensorRead>(v).expect("назад"), r);
        let u = SensorRead::Unsupported {
            cpu: "Intel".into(),
        };
        let back: SensorRead =
            serde_json::from_value(serde_json::to_value(&u).expect("json")).expect("назад");
        assert_eq!(back, u);
    }
}
