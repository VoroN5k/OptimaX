//! Служби Windows за трейтом [`ServiceApi`]: лише стан, зупинка і запуск.
//! Тип запуску (автоматичний/вручну) OptimaX не змінює.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::WinServices;

/// Служби, які режим гри може **тимчасово** зупиняти. Будь-яка інша — відмова.
/// Оновлення, Defender, брандмауер, телеметрія сюди не входять і не ввійдуть за замовчуванням.
pub const STOPPABLE_SERVICES: &[&str] = &["WSearch", "SysMain"];

/// Чи можна тимчасово зупиняти службу.
pub fn is_stoppable_service(name: &str) -> bool {
    STOPPABLE_SERVICES
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}

/// Стан служби.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    /// Працює.
    Running,
    /// Зупинена.
    Stopped,
    /// Перехідний чи інший стан (код SERVICE_*).
    Other(u32),
}

/// Керування службами.
pub trait ServiceApi {
    /// Поточний стан.
    fn state(&self, name: &str) -> Result<ServiceState, CoreError>;
    /// Зупиняє службу й чекає, доки вона зупиниться.
    fn stop(&self, name: &str) -> Result<(), CoreError>;
    /// Запускає службу й чекає, доки вона запрацює.
    fn start(&self, name: &str) -> Result<(), CoreError>;
}

/// Служби в пам'яті для тестів.
#[derive(Debug, Default)]
pub struct MemServices {
    states: Mutex<HashMap<String, ServiceState>>,
}

impl MemServices {
    /// Служби з початковими станами.
    pub fn new(initial: &[(&str, ServiceState)]) -> Self {
        Self {
            states: Mutex::new(
                initial
                    .iter()
                    .map(|(n, s)| (n.to_ascii_lowercase(), *s))
                    .collect(),
            ),
        }
    }

    fn set(&self, name: &str, state: ServiceState) -> Result<(), CoreError> {
        let mut st = self.states.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = st
            .get_mut(&name.to_ascii_lowercase())
            .ok_or_else(|| CoreError::NotFound(format!("служба {name}")))?;
        *slot = state;
        Ok(())
    }
}

impl ServiceApi for MemServices {
    fn state(&self, name: &str) -> Result<ServiceState, CoreError> {
        self.states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&name.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| CoreError::NotFound(format!("служба {name}")))
    }

    fn stop(&self, name: &str) -> Result<(), CoreError> {
        self.set(name, ServiceState::Stopped)
    }

    fn start(&self, name: &str) -> Result<(), CoreError> {
        self.set(name, ServiceState::Running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        assert!(is_stoppable_service("wsearch"));
        assert!(is_stoppable_service("SysMain"));
        for forbidden in ["wuauserv", "WinDefend", "mpssvc", "DiagTrack", "BITS"] {
            assert!(!is_stoppable_service(forbidden), "{forbidden}");
        }
    }
}
