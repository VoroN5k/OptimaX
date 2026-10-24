//! Мережа для ігор: який адаптер несе інтернет (Ethernet / Wi-Fi / VPN), якість Wi-Fi,
//! затримка, джитер і втрати до роутера та до вузла за межами дому.
//!
//! Замір запускається лише явно (кнопкою): пінги йдуть на роутер і на вузол, який бачить
//! користувач. Мережевих «твіків» реєстру не пропонуємо — у сучасних іграх вони не мають
//! виміряного ефекту.
//!
//! `unsafe` дозволено лише в платформному модулі `imp` (FFI до IP Helper і WLAN API).

pub mod model;
pub mod rules;
pub mod runner;

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "win/mod.rs"]
mod imp;

#[cfg(not(windows))]
#[path = "stub.rs"]
mod imp;

pub use model::{Adapter, LinkKind, NetworkInfo, PingStats, WifiBand, WifiInfo};
pub use rules::{analyze, analyze_link, Measurement, Verdict};
pub use runner::{resolve, run_test, CancelToken, EchoApi, TestOptions};

/// Помилки модуля мережі.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// Адресу не вдалося визначити (немає IPv4 або DNS не відповів).
    #[error("не вдалося визначити адресу «{0}»")]
    Resolve(String),
    /// Виклик WinAPI повернув помилку.
    #[error("{api} повернув помилку {code}")]
    Os {
        /// Назва функції.
        api: &'static str,
        /// Код помилки.
        code: u32,
    },
    /// Недоступно на цій платформі.
    #[error("не підтримується: {0}")]
    Unsupported(&'static str),
}

/// Підключені мережеві адаптери з IPv4 і Wi-Fi-подробицями.
pub fn network_info() -> Result<NetworkInfo, NetworkError> {
    imp::network_info()
}

/// Пінг через системний ICMP API Windows (без сторонніх програм).
pub fn system_echo() -> Box<dyn EchoApi> {
    Box::new(imp::IcmpEcho)
}
