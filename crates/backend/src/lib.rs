//! Бекенд OptimaX без Tauri (етап 2 з `docs/SERVICE.md`): те, що потребує прав адміністратора й
//! тому виконується в службі, а без служби — у самій програмі.
//!
//! Поки це датчики CPU через PawnIO ([`sensors`]). Запис гри — наступним кроком.

#![forbid(unsafe_code)]

pub mod sensors;

#[cfg(windows)]
pub use self::service::handler;

#[cfg(windows)]
mod service {
    use crate::sensors::SensorCache;
    use optimax_service::host::Handler;
    use optimax_service::protocol::{Command, Reply};
    use std::sync::Arc;

    fn json<T: serde::Serialize>(v: &T) -> Reply {
        serde_json::to_value(v).map_or_else(|e| Reply::Error(e.to_string()), Reply::Json)
    }

    /// Виконавець команд служби. Модулі PawnIO відкриваються при першому запиті й тримаються
    /// відкритими весь час роботи служби.
    pub fn handler() -> Handler {
        let sensors = Arc::new(SensorCache::new());
        Arc::new(move |cmd: &Command| match cmd {
            Command::Sensors => json(&sensors.read()),
            Command::CpuTemp => json(&sensors.temp()),
            // `ping` служба відповідає сама.
            Command::Ping => Reply::Error("ping обробляє служба".into()),
        })
    }
}
