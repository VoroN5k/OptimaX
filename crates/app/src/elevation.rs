//! Права при запуску (етап 3д з `docs/SERVICE.md`). Маніфест — `asInvoker`: якщо служба OptimaX
//! працює й приймає цей exe, програма працює без прав адміністратора й без запиту UAC; інакше
//! перезапускається з правами (один запит UAC, як раніше з `requireAdministrator`).

use optimax_core::elevate::{is_cancelled, run_elevated};
use optimax_service::client::Client;
use optimax_service::install::{self, SvcState};
use optimax_service::{same_file, service_exe};
use std::time::{Duration, Instant};

/// Скільки чекати службу, що саме запускається.
const PENDING_WAIT: Duration = Duration::from_secs(10);

/// Що робити при запуску.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Працювати в цьому процесі (є права або служба).
    Here,
    /// Запущено копію з правами — цей процес завершується.
    Relaunched,
    /// Користувач відмовився в UAC — завершуємось (як з `requireAdministrator`).
    Declined,
}

/// Програма запущена з exe служби (інакше служба з'єднання не прийме).
pub fn from_service_exe() -> bool {
    std::env::current_exe()
        .ok()
        .zip(service_exe())
        .is_some_and(|(me, svc)| same_file(&me, &svc))
}

/// Служба працює, приймає цей exe й відповідає з тим самим протоколом. Службу, що саме
/// запускається (напр. одразу після входу в Windows), трохи чекаємо.
pub fn service_ready() -> bool {
    if !from_service_exe() {
        return false;
    }
    let deadline = Instant::now() + PENDING_WAIT;
    loop {
        match install::state() {
            Ok(SvcState::Running) => break,
            Ok(SvcState::Pending) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(300));
            }
            _ => return false,
        }
    }
    match Client::connect().and_then(|mut c| c.ping()) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("служба OptimaX не відповіла при запуску: {e}");
            false
        }
    }
}

/// Розробник у debug-збірці може відкрити вікно без UAC (`OPTIMAX_DEV_AS_INVOKER=1`): зміни
/// системи тоді не працюватимуть — це очікувано. Release змінну ігнорує.
fn dev_as_invoker() -> bool {
    cfg!(debug_assertions) && std::env::var("OPTIMAX_DEV_AS_INVOKER").is_ok_and(|v| v == "1")
}

/// Аргументи цього запуску (без exe) — для копії з правами.
fn own_args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

/// Перезапускає програму з правами адміністратора (UAC). `Ok(())` — копію запущено.
pub fn relaunch() -> Result<(), optimax_core::CoreError> {
    run_elevated(&own_args(), false).map(|_| ())
}

/// Вирішує, чи працювати в цьому процесі.
pub fn decide() -> Start {
    if optimax_core::process::is_elevated() || dev_as_invoker() || service_ready() {
        return Start::Here;
    }
    match relaunch() {
        Ok(()) => Start::Relaunched,
        Err(e) if is_cancelled(&e) => Start::Declined,
        Err(e) => {
            // UAC недоступний (вимкнено політикою тощо) — працюємо як є: читання працює, зміни
            // скажуть про брак прав.
            tracing::warn!("не вдалося перезапуститися з правами адміністратора: {e}");
            Start::Here
        }
    }
}
