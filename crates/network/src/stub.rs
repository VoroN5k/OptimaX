//! Заглушка для не-Windows: адаптерів і пінгу немає, логіку можна тестувати.

use crate::model::NetworkInfo;
use crate::runner::EchoApi;
use crate::NetworkError;
use std::net::Ipv4Addr;
use std::time::Duration;

pub(crate) fn network_info() -> Result<NetworkInfo, NetworkError> {
    Err(NetworkError::Unsupported(
        "мережеві адаптери (лише Windows)",
    ))
}

pub(crate) struct IcmpEcho;

impl EchoApi for IcmpEcho {
    fn echo(&self, _target: Ipv4Addr, _timeout: Duration) -> Option<f64> {
        None
    }
}
