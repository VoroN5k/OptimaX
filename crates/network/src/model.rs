//! Модель мережевого з'єднання: адаптер, Wi-Fi, статистика пінгу. Чисті функції — з тестами.

use serde::{Deserialize, Serialize};

/// Тип з'єднання.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Дротове (Ethernet).
    Ethernet,
    /// Бездротове (Wi-Fi).
    Wifi,
    /// Віртуальний адаптер: VPN, тунель (Radmin VPN, Tailscale, WireGuard…).
    Virtual,
    /// Інше (мобільний модем тощо).
    Other,
}

/// Код типу інтерфейсу IANA (`IfType`): Ethernet.
pub const IF_TYPE_ETHERNET: u32 = 6;
/// Код типу інтерфейсу IANA: Wi-Fi (IEEE 802.11).
pub const IF_TYPE_WIFI: u32 = 71;

/// Тип з'єднання за кодом інтерфейсу і тим, чи це фізичний пристрій
/// (`HardwareInterface` у `MIB_IF_ROW2`). VPN-адаптери часто «прикидаються» Ethernet,
/// тому ознака апаратного інтерфейсу важливіша за код типу.
pub fn classify(if_type: u32, hardware: bool) -> LinkKind {
    match (hardware, if_type) {
        (false, _) => LinkKind::Virtual,
        (true, IF_TYPE_WIFI) => LinkKind::Wifi,
        (true, IF_TYPE_ETHERNET) => LinkKind::Ethernet,
        (true, _) => LinkKind::Other,
    }
}

/// Діапазон Wi-Fi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum WifiBand {
    /// 2,4 ГГц: далекобійний, але перевантажений і схильний до завад.
    Ghz2_4,
    /// 5 ГГц.
    Ghz5,
    /// 6 ГГц (Wi-Fi 6E / 7).
    Ghz6,
}

/// Діапазон і канал за центральною частотою (кГц, як `ulChCenterFrequency` у WLAN API).
pub fn band_and_channel(center_khz: u32) -> Option<(WifiBand, u32)> {
    let mhz = center_khz / 1000;
    match mhz {
        2484 => Some((WifiBand::Ghz2_4, 14)),
        2412..=2472 => Some((WifiBand::Ghz2_4, (mhz - 2407) / 5)),
        5150..=5895 => Some((WifiBand::Ghz5, (mhz - 5000) / 5)),
        5925..=7125 => Some((WifiBand::Ghz6, (mhz - 5950) / 5)),
        _ => None,
    }
}

/// Стандарт Wi-Fi за `DOT11_PHY_TYPE`.
pub fn wifi_standard(phy: i32) -> Option<&'static str> {
    Some(match phy {
        4 => "802.11a",
        5 => "802.11b",
        6 => "802.11g",
        7 => "802.11n (Wi-Fi 4)",
        8 => "802.11ac (Wi-Fi 5)",
        10 => "802.11ax (Wi-Fi 6/6E)",
        11 => "802.11be (Wi-Fi 7)",
        _ => return None,
    })
}

/// Стан Wi-Fi-з'єднання.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WifiInfo {
    /// Назва мережі (може бути недоступна без дозволу на розташування).
    pub ssid: Option<String>,
    /// Якість сигналу за Windows, 0–100 % (`None` — Windows не повідомив).
    pub signal_pct: Option<u32>,
    /// Рівень сигналу, дБм.
    pub rssi_dbm: Option<i32>,
    /// Діапазон.
    pub band: Option<WifiBand>,
    /// Канал.
    pub channel: Option<u32>,
    /// Стандарт (802.11ax тощо).
    pub standard: Option<String>,
    /// Швидкість прийому / передачі на каналі, Мбіт/с.
    pub rx_mbps: u32,
    /// Швидкість передачі, Мбіт/с.
    pub tx_mbps: u32,
    /// Windows не дав подробиць (діапазон, канал, назву) без дозволу на розташування
    /// (Windows 11 24H2+: «Дозволити класичним програмам доступ до розташування»).
    pub details_blocked: bool,
}

/// Мережевий адаптер.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Adapter {
    /// Назва в системі («Ethernet 2»).
    pub name: String,
    /// Опис (модель).
    pub description: String,
    /// Тип з'єднання.
    pub kind: LinkKind,
    /// Швидкість з'єднання, Мбіт/с.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub link_mbps: Option<u64>,
    /// IPv4-адреса шлюзу (роутера).
    pub gateway: Option<String>,
    /// IPv4-адреси DNS-серверів.
    pub dns: Vec<String>,
    /// Через цей адаптер іде маршрут в інтернет за замовчуванням.
    pub carries_internet: bool,
    /// Wi-Fi (для бездротових).
    pub wifi: Option<WifiInfo>,
}

/// Знімок мережевих адаптерів.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NetworkInfo {
    /// Підключені адаптери з IPv4.
    pub adapters: Vec<Adapter>,
}

impl NetworkInfo {
    /// Адаптер, через який іде інтернет.
    pub fn internet(&self) -> Option<&Adapter> {
        self.adapters.iter().find(|a| a.carries_internet)
    }

    /// Ціль за замовчуванням для заміру «за межами дому»: перший DNS-сервер, що не є
    /// самим роутером і не з приватної мережі. `None` — DNS вказує на роутер (типово), тоді
    /// користувач вводить вузол сам.
    pub fn default_remote(&self) -> Option<String> {
        let a = self.internet()?;
        a.dns
            .iter()
            .find(|d| Some(*d) != a.gateway.as_ref() && !is_private(d))
            .cloned()
    }
}

/// Приватна / локальна IPv4-адреса (RFC 1918, CGNAT, loopback, link-local).
pub fn is_private(addr: &str) -> bool {
    let Ok(ip) = addr.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    let [a, b, ..] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || (a == 100 && (64..128).contains(&b))
}

/// Статистика пінгу до однієї цілі.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PingStats {
    /// Ціль (IP).
    pub target: String,
    /// Надіслано.
    pub sent: u32,
    /// Отримано відповідей.
    pub received: u32,
    /// Втрати, %.
    pub loss_pct: f64,
    /// Мінімум, мс.
    pub min_ms: Option<f64>,
    /// Середнє, мс.
    pub avg_ms: Option<f64>,
    /// Медіана, мс.
    pub median_ms: Option<f64>,
    /// 95-й перцентиль, мс.
    pub p95_ms: Option<f64>,
    /// Максимум, мс.
    pub max_ms: Option<f64>,
    /// Джитер: середня різниця між сусідніми відповідями, мс (як у RFC 3550, без згладжування).
    pub jitter_ms: Option<f64>,
}

impl PingStats {
    /// Статистика за рядом: `Some(мс)` — відповідь, `None` — втрата.
    pub fn from_samples(target: impl Into<String>, samples: &[Option<f64>]) -> Self {
        let rtts: Vec<f64> = samples.iter().flatten().copied().collect();
        let sent = u32::try_from(samples.len()).unwrap_or(u32::MAX);
        let received = u32::try_from(rtts.len()).unwrap_or(u32::MAX);
        let loss_pct = if sent == 0 {
            0.0
        } else {
            f64::from(sent - received) / f64::from(sent) * 100.0
        };
        let mut sorted = rtts.clone();
        sorted.sort_by(f64::total_cmp);
        let pct = |p: f64| -> Option<f64> {
            match sorted.len() {
                0 => None,
                n => {
                    let rank = p / 100.0 * (n - 1) as f64;
                    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
                    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64))
                }
            }
        };
        let jitter = (rtts.len() >= 2).then(|| {
            rtts.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (rtts.len() - 1) as f64
        });
        Self {
            target: target.into(),
            sent,
            received,
            loss_pct,
            min_ms: sorted.first().copied(),
            avg_ms: (!rtts.is_empty()).then(|| rtts.iter().sum::<f64>() / rtts.len() as f64),
            median_ms: pct(50.0),
            p95_ms: pct(95.0),
            max_ms: sorted.last().copied(),
            jitter_ms: jitter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_prefers_hardware_flag() {
        assert_eq!(classify(IF_TYPE_ETHERNET, true), LinkKind::Ethernet);
        assert_eq!(classify(IF_TYPE_WIFI, true), LinkKind::Wifi);
        // VPN-адаптер, що видає себе за Ethernet.
        assert_eq!(classify(IF_TYPE_ETHERNET, false), LinkKind::Virtual);
        assert_eq!(classify(243, true), LinkKind::Other);
    }

    #[test]
    fn bands_and_channels() {
        assert_eq!(band_and_channel(2_412_000), Some((WifiBand::Ghz2_4, 1)));
        assert_eq!(band_and_channel(2_437_000), Some((WifiBand::Ghz2_4, 6)));
        assert_eq!(band_and_channel(2_484_000), Some((WifiBand::Ghz2_4, 14)));
        assert_eq!(band_and_channel(5_180_000), Some((WifiBand::Ghz5, 36)));
        assert_eq!(band_and_channel(5_745_000), Some((WifiBand::Ghz5, 149)));
        assert_eq!(band_and_channel(5_955_000), Some((WifiBand::Ghz6, 1)));
        assert_eq!(band_and_channel(6_115_000), Some((WifiBand::Ghz6, 33)));
        assert_eq!(band_and_channel(900_000), None);
    }

    #[test]
    fn standards() {
        assert_eq!(wifi_standard(10), Some("802.11ax (Wi-Fi 6/6E)"));
        assert_eq!(wifi_standard(0), None);
    }

    #[test]
    fn private_addresses() {
        assert!(is_private("192.168.1.1"));
        assert!(is_private("10.0.0.1"));
        assert!(is_private("100.100.100.100")); // CGNAT / Tailscale
        assert!(!is_private("1.1.1.1"));
        assert!(!is_private("26.0.0.1"));
        assert!(!is_private("не адреса"));
    }

    #[test]
    fn ping_stats_basic() {
        let s = PingStats::from_samples("x", &[Some(10.0), Some(12.0), None, Some(11.0)]);
        assert_eq!((s.sent, s.received), (4, 3));
        assert!((s.loss_pct - 25.0).abs() < 1e-9);
        assert_eq!(s.min_ms, Some(10.0));
        assert_eq!(s.max_ms, Some(12.0));
        assert_eq!(s.median_ms, Some(11.0));
        assert!((s.avg_ms.unwrap_or_default() - 11.0).abs() < 1e-9);
        // |12−10| і |11−12| → 1,5 мс.
        assert!((s.jitter_ms.unwrap_or_default() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn ping_stats_all_lost_or_empty() {
        let s = PingStats::from_samples("x", &[None, None]);
        assert_eq!(s.received, 0);
        assert_eq!(s.loss_pct, 100.0);
        assert!(s.avg_ms.is_none() && s.jitter_ms.is_none());
        let e = PingStats::from_samples("x", &[]);
        assert_eq!(e.loss_pct, 0.0);
    }

    fn info(gateway: &str, dns: &[&str]) -> NetworkInfo {
        NetworkInfo {
            adapters: vec![Adapter {
                name: "Ethernet".into(),
                description: String::new(),
                kind: LinkKind::Ethernet,
                link_mbps: Some(1000),
                gateway: Some(gateway.into()),
                dns: dns.iter().map(|d| d.to_string()).collect(),
                carries_internet: true,
                wifi: None,
            }],
        }
    }

    #[test]
    fn default_remote_skips_router_and_private_dns() {
        assert_eq!(info("192.168.1.1", &["192.168.1.1"]).default_remote(), None);
        assert_eq!(
            info("192.168.1.1", &["192.168.1.1", "8.8.8.8"]).default_remote(),
            Some("8.8.8.8".into())
        );
        assert_eq!(info("192.168.1.1", &["10.0.0.53"]).default_remote(), None);
    }
}
