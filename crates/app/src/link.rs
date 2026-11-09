//! Куди йти по дані, що потребують прав адміністратора: у службу OptimaX (якщо вона працює й
//! прийме цей exe) чи в сам процес програми. Служба відмовила — тихо читаємо самі.

use optimax_backend::sensors::{SensorCache, SensorRead};
use optimax_service::client::Client;
use optimax_service::install::{self, SvcState};
use optimax_service::protocol::Command;
use optimax_service::{same_file, service_exe};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Як часто перепитувати диспетчер служб, чи служба доступна.
const RECHECK: Duration = Duration::from_secs(10);

/// Вибір шляху й локальний кеш датчиків.
#[derive(Debug, Default)]
pub struct Link {
    local: SensorCache,
    usable: Mutex<Option<(Instant, bool)>>,
}

impl Link {
    /// Нічого не відкриває, доки не попросять.
    pub fn new() -> Self {
        Self::default()
    }

    /// Служба працює, і ця програма запущена з її exe (інакше служба з'єднання не прийме).
    pub fn service_usable(&self) -> bool {
        let mut slot = self.usable.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, v)) = *slot {
            if at.elapsed() < RECHECK {
                return v;
            }
        }
        let v = install::state().is_ok_and(|s| s == SvcState::Running)
            && std::env::current_exe()
                .ok()
                .zip(service_exe())
                .is_some_and(|(me, svc)| same_file(&me, &svc));
        *slot = Some((Instant::now(), v));
        v
    }

    fn forget_service(&self, why: &dyn std::fmt::Display) {
        tracing::warn!("служба OptimaX не відповіла, читаю сам: {why}");
        *self.usable.lock().unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), false));
    }

    fn via_service<T: serde::de::DeserializeOwned>(&self, cmd: Command) -> Option<T> {
        if !self.service_usable() {
            return None;
        }
        match Client::connect().and_then(|mut c| c.json::<T>(cmd)) {
            Ok(v) => Some(v),
            Err(e) => {
                self.forget_service(&e);
                None
            }
        }
    }

    /// Датчики PawnIO і чи прочитано через службу.
    pub fn sensors(&self) -> (SensorRead, bool) {
        match self.via_service(Command::Sensors) {
            Some(r) => (r, true),
            None => (self.local.read(), false),
        }
    }

    /// Лише температура CPU.
    pub fn cpu_temp(&self) -> Option<f32> {
        match self.via_service::<Option<f32>>(Command::CpuTemp) {
            Some(t) => t,
            None => self.local.temp(),
        }
    }
}
