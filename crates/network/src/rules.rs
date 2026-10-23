//! Висновки про мережу за виміряними даними. Чисті функції — покриті тестами.
//!
//! Головне питання для гравця: «де саме проблема?». Пінг до роутера показує домашню мережу
//! (Wi-Fi, кабель, роутер), пінг до вузла за межами дому — провайдера й маршрут. Мережевих
//! «твіків» реєстру не пропонуємо: у сучасних іграх (UDP, вимкнений Nagle) вони не мають
//! виміряного ефекту.

use crate::model::{Adapter, LinkKind, PingStats, WifiBand};
use optimax_core::{Category, Finding, FindingBuilder, Impact, Text};
use serde::{Deserialize, Serialize};

/// Менше відповідей — висновків про стабільність не робимо.
pub const MIN_REPLIES: u32 = 30;
/// Джитер до роутера, вище якого домашня мережа нестабільна, мс. Кабель дає < 0,5 мс,
/// добрий Wi-Fi — 1–2 мс.
pub const LOCAL_JITTER_MS: f64 = 3.0;
/// Втрати, вище яких з'єднання нестабільне, %.
pub const LOSS_PCT: f64 = 1.0;
/// Джитер до зовнішнього вузла, вище якого маршрут нестабільний, мс.
pub const REMOTE_JITTER_MS: f64 = 8.0;
/// Медіанна затримка до зовнішнього вузла, вище якої вона помітна в онлайн-іграх, мс.
pub const HIGH_LATENCY_MS: f64 = 80.0;
/// Якість сигналу Wi-Fi, нижче якої з'єднання слабке, %.
pub const WEAK_SIGNAL_PCT: u32 = 50;
/// Рівень сигналу, нижче якого з'єднання слабке, дБм.
pub const WEAK_RSSI_DBM: i32 = -70;

/// Головний висновок заміру.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Стабільно.
    Ok,
    /// Проблема в домашній мережі.
    Local,
    /// Удома стабільно, проблема далі (провайдер, маршрут, сервер).
    Remote,
    /// Замало даних (роутер не відповідає на пінг тощо).
    Insufficient,
}

/// Результат заміру для правил.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Measurement {
    /// Пінг до роутера.
    pub gateway: Option<PingStats>,
    /// Пінг до вузла за межами дому.
    pub remote: Option<PingStats>,
}

fn unstable(s: &PingStats, jitter_limit: f64) -> bool {
    s.received >= MIN_REPLIES
        && (s.loss_pct > LOSS_PCT || s.jitter_ms.is_some_and(|j| j > jitter_limit))
}

fn enough(s: Option<&PingStats>) -> bool {
    s.is_some_and(|s| s.received >= MIN_REPLIES)
}

/// Впевненість за кількістю відповідей: 30 → 60 %, 300+ → 90 %.
fn confidence(s: &PingStats) -> u8 {
    (55.0 + f64::from(s.received.min(300)) / 300.0 * 35.0).round() as u8
}

fn ms(v: Option<f64>) -> Text {
    Text::new("value.ms").arg("v", v.map_or_else(|| "—".into(), |v| format!("{v:.1}")))
}

fn pct(v: f64) -> Text {
    Text::new("value.pct").arg("v", format!("{v:.1}"))
}

fn ping_evidence(b: FindingBuilder, s: &PingStats, prefix: &'static str) -> FindingBuilder {
    let (avg, jitter, loss, p95) = match prefix {
        "gw" => (
            "net.ev.gw_avg",
            "net.ev.gw_jitter",
            "net.ev.gw_loss",
            "net.ev.gw_p95",
        ),
        _ => (
            "net.ev.remote_avg",
            "net.ev.remote_jitter",
            "net.ev.remote_loss",
            "net.ev.remote_p95",
        ),
    };
    b.evidence(Text::new(avg), ms(s.avg_ms))
        .evidence(Text::new(jitter), ms(s.jitter_ms))
        .evidence(Text::new(p95), ms(s.p95_ms))
        .evidence(Text::new(loss), pct(s.loss_pct))
}

/// Головний висновок.
pub fn verdict(m: &Measurement) -> Verdict {
    let gw = m.gateway.as_ref();
    let remote = m.remote.as_ref();
    if !enough(gw) && !enough(remote) {
        return Verdict::Insufficient;
    }
    if gw.is_some_and(|g| unstable(g, LOCAL_JITTER_MS)) {
        return Verdict::Local;
    }
    if remote.is_some_and(|r| unstable(r, REMOTE_JITTER_MS)) {
        // Якщо роутер не виміряно, не можемо сказати, що вдома все добре.
        return if enough(gw) {
            Verdict::Remote
        } else {
            Verdict::Insufficient
        };
    }
    Verdict::Ok
}

/// Підсумкова знахідка.
pub fn summary(m: &Measurement, link: Option<&Adapter>) -> Finding {
    let v = verdict(m);
    let id = match v {
        Verdict::Ok => "summary.ok",
        Verdict::Local => "summary.local",
        Verdict::Remote => "summary.remote",
        Verdict::Insufficient => "summary.insufficient",
    };
    let conf = m
        .gateway
        .as_ref()
        .or(m.remote.as_ref())
        .map_or(100, confidence);
    let mut b = FindingBuilder::new("net", id, Category::Summary, Impact::Info, conf).arg(
        "link",
        match link.map(|l| l.kind) {
            Some(LinkKind::Wifi) => "Wi-Fi",
            Some(LinkKind::Ethernet) => "Ethernet",
            Some(LinkKind::Virtual) => "VPN",
            _ => "—",
        },
    );
    if let Some(g) = &m.gateway {
        b = ping_evidence(b, g, "gw");
    }
    if let Some(r) = &m.remote {
        b = ping_evidence(b, r, "remote");
    }
    b.build()
}

/// Домашня мережа нестабільна (джитер / втрати до роутера).
pub fn local_unstable(m: &Measurement, link: Option<&Adapter>) -> Option<Finding> {
    let g = m
        .gateway
        .as_ref()
        .filter(|g| unstable(g, LOCAL_JITTER_MS))?;
    let severe = g.loss_pct > 2.0 * LOSS_PCT || g.jitter_ms.is_some_and(|j| j > 10.0);
    let id = if link.is_some_and(|l| l.kind == LinkKind::Wifi) {
        "local_unstable.wifi"
    } else {
        "local_unstable.wired"
    };
    let impact = if severe {
        Impact::High
    } else {
        Impact::Moderate
    };
    Some(
        ping_evidence(
            FindingBuilder::new("net", id, Category::Network, impact, confidence(g)).advice(),
            g,
            "gw",
        )
        .build(),
    )
}

/// Удома стабільно, а далі — ні: провайдер, маршрут або сам сервер.
pub fn remote_unstable(m: &Measurement) -> Option<Finding> {
    let g = m.gateway.as_ref().filter(|g| g.received >= MIN_REPLIES)?;
    if unstable(g, LOCAL_JITTER_MS) {
        return None;
    }
    let r = m
        .remote
        .as_ref()
        .filter(|r| unstable(r, REMOTE_JITTER_MS))?;
    Some(
        ping_evidence(
            ping_evidence(
                FindingBuilder::new(
                    "net",
                    "remote_unstable",
                    Category::Network,
                    Impact::Moderate,
                    confidence(r),
                )
                .arg("target", &r.target)
                .advice(),
                r,
                "remote",
            ),
            g,
            "gw",
        )
        .build(),
    )
}

/// Велика затримка до вузла (далекий сервер) — інформаційно, OptimaX її не змінить.
pub fn high_latency(m: &Measurement) -> Option<Finding> {
    let r = m.remote.as_ref().filter(|r| r.received >= MIN_REPLIES)?;
    let median = r.median_ms.filter(|v| *v > HIGH_LATENCY_MS)?;
    Some(
        FindingBuilder::new(
            "net",
            "high_latency",
            Category::Network,
            Impact::Info,
            confidence(r),
        )
        .arg("target", &r.target)
        .arg("ms", format!("{median:.0}"))
        .evidence(Text::new("net.ev.remote_median"), ms(Some(median)))
        .build(),
    )
}

/// Wi-Fi на 2,4 ГГц. Помірний вплив, якщо до роутера ще й нестабільно, інакше — інформаційно.
pub fn wifi_2_4ghz(m: &Measurement, link: Option<&Adapter>) -> Option<Finding> {
    let w = link.filter(|l| l.kind == LinkKind::Wifi)?.wifi.as_ref()?;
    if w.band != Some(WifiBand::Ghz2_4) {
        return None;
    }
    let shaky = m
        .gateway
        .as_ref()
        .is_some_and(|g| unstable(g, LOCAL_JITTER_MS));
    let impact = if shaky {
        Impact::Moderate
    } else {
        Impact::Info
    };
    let mut b = FindingBuilder::new("net", "wifi_2_4ghz", Category::Network, impact, 90).advice();
    if let Some(c) = w.channel {
        b = b.evidence(Text::new("net.ev.channel"), Text::raw(c));
    }
    Some(b.build())
}

/// Слабкий сигнал Wi-Fi.
pub fn wifi_weak(link: Option<&Adapter>) -> Option<Finding> {
    let w = link.filter(|l| l.kind == LinkKind::Wifi)?.wifi.as_ref()?;
    let weak_rssi = w.rssi_dbm.is_some_and(|r| r < WEAK_RSSI_DBM);
    let weak_quality = w.signal_pct.is_some_and(|q| q < WEAK_SIGNAL_PCT);
    if !weak_quality && !weak_rssi {
        return None;
    }
    let mut b =
        FindingBuilder::new("net", "wifi_weak", Category::Network, Impact::Moderate, 85).advice();
    if let Some(q) = w.signal_pct {
        b = b.evidence(
            Text::new("net.ev.signal"),
            Text::new("value.pct").arg("v", q),
        );
    }
    if let Some(r) = w.rssi_dbm {
        b = b.evidence(Text::new("net.ev.rssi"), Text::new("value.dbm").arg("v", r));
    }
    Some(b.build())
}

/// Інтернет іде через віртуальний адаптер (VPN): маршрут довший, затримка вища.
pub fn vpn_route(link: Option<&Adapter>) -> Option<Finding> {
    let l = link.filter(|l| l.kind == LinkKind::Virtual)?;
    Some(
        FindingBuilder::new("net", "vpn_route", Category::Network, Impact::Moderate, 90)
            .arg("name", &l.name)
            .advice()
            .evidence(Text::new("net.ev.adapter"), Text::raw(&l.description))
            .build(),
    )
}

/// Дротове з'єднання на швидкості нижче 1 Гбіт/с — часто ознака пошкодженого кабелю чи порту.
/// На затримку майже не впливає, тому інформаційно.
pub fn slow_link(link: Option<&Adapter>) -> Option<Finding> {
    let l = link.filter(|l| l.kind == LinkKind::Ethernet)?;
    let mbps = l.link_mbps.filter(|m| *m < 1000)?;
    Some(
        FindingBuilder::new("net", "slow_link", Category::Network, Impact::Info, 80)
            .arg("mbps", mbps)
            .advice()
            .evidence(
                Text::new("net.ev.link_speed"),
                Text::new("value.mbps").arg("v", mbps),
            )
            .build(),
    )
}

/// Усі висновки: підсумок першим, далі за впливом.
pub fn analyze(m: &Measurement, link: Option<&Adapter>) -> Vec<Finding> {
    let mut found: Vec<Finding> = [
        local_unstable(m, link),
        remote_unstable(m),
        wifi_2_4ghz(m, link),
        wifi_weak(link),
        vpn_route(link),
        high_latency(m),
        slow_link(link),
    ]
    .into_iter()
    .flatten()
    .collect();
    found.sort_by(|a, b| {
        b.impact
            .cmp(&a.impact)
            .then(b.confidence.cmp(&a.confidence))
    });
    let mut out = vec![summary(m, link)];
    out.extend(found);
    out
}

/// Висновки без заміру — лише за станом адаптера (до натискання «Почати замір»).
pub fn analyze_link(link: Option<&Adapter>) -> Vec<Finding> {
    let none = Measurement {
        gateway: None,
        remote: None,
    };
    [
        wifi_2_4ghz(&none, link),
        wifi_weak(link),
        vpn_route(link),
        slow_link(link),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WifiInfo;
    use optimax_core::Catalog;

    fn series(base: f64, spread: f64, n: usize, lost_every: usize) -> Vec<Option<f64>> {
        (0..n)
            .map(|i| {
                if lost_every > 0 && i % lost_every == 0 {
                    None
                } else {
                    Some(base + if i % 2 == 0 { spread } else { 0.0 })
                }
            })
            .collect()
    }

    fn stats(target: &str, base: f64, spread: f64, lost_every: usize) -> PingStats {
        PingStats::from_samples(target, &series(base, spread, 300, lost_every))
    }

    fn adapter(kind: LinkKind, wifi: Option<WifiInfo>) -> Adapter {
        Adapter {
            name: "Net".into(),
            description: "NIC".into(),
            kind,
            link_mbps: Some(1000),
            gateway: Some("192.168.1.1".into()),
            dns: vec![],
            carries_internet: true,
            wifi,
        }
    }

    fn wifi(band: WifiBand, signal: u32) -> WifiInfo {
        WifiInfo {
            ssid: Some("Home".into()),
            signal_pct: Some(signal),
            rssi_dbm: Some(-55),
            band: Some(band),
            channel: Some(6),
            standard: None,
            rx_mbps: 300,
            tx_mbps: 300,
            details_blocked: false,
        }
    }

    fn ids(f: &[Finding]) -> Vec<&str> {
        f.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn stable_wired_network_is_ok() {
        let m = Measurement {
            gateway: Some(stats("192.168.1.1", 0.4, 0.1, 0)),
            remote: Some(stats("8.8.8.8", 12.0, 1.0, 0)),
        };
        assert_eq!(verdict(&m), Verdict::Ok);
        let f = analyze(&m, Some(&adapter(LinkKind::Ethernet, None)));
        assert_eq!(ids(&f), vec!["summary.ok"]);
    }

    #[test]
    fn jittery_wifi_is_local_problem() {
        let m = Measurement {
            gateway: Some(stats("192.168.1.1", 3.0, 12.0, 0)),
            remote: Some(stats("8.8.8.8", 20.0, 12.0, 0)),
        };
        assert_eq!(verdict(&m), Verdict::Local);
        let link = adapter(LinkKind::Wifi, Some(wifi(WifiBand::Ghz2_4, 80)));
        let f = analyze(&m, Some(&link));
        assert_eq!(f[0].id, "summary.local");
        assert!(ids(&f).contains(&"local_unstable.wifi"));
        // 2,4 ГГц при нестабільному Wi-Fi — помірний вплив.
        let band = f.iter().find(|x| x.id == "wifi_2_4ghz").unwrap();
        assert_eq!(band.impact, Impact::Moderate);
        // Проблема вдома — провайдера не звинувачуємо.
        assert!(!ids(&f).contains(&"remote_unstable"));
    }

    #[test]
    fn local_loss_is_high_impact() {
        let m = Measurement {
            gateway: Some(stats("192.168.1.1", 0.5, 0.1, 20)), // 5 % втрат
            remote: None,
        };
        let f = local_unstable(&m, Some(&adapter(LinkKind::Ethernet, None))).unwrap();
        assert_eq!(f.id, "local_unstable.wired");
        assert_eq!(f.impact, Impact::High);
    }

    #[test]
    fn stable_home_unstable_route_is_remote() {
        let m = Measurement {
            gateway: Some(stats("192.168.1.1", 0.4, 0.1, 0)),
            remote: Some(stats("1.2.3.4", 30.0, 20.0, 0)),
        };
        assert_eq!(verdict(&m), Verdict::Remote);
        let f = analyze(&m, Some(&adapter(LinkKind::Ethernet, None)));
        assert_eq!(ids(&f)[..2], ["summary.remote", "remote_unstable"]);
    }

    #[test]
    fn remote_problem_without_router_data_is_not_blamed_on_isp() {
        let m = Measurement {
            gateway: Some(PingStats::from_samples("192.168.1.1", &[None; 50])),
            remote: Some(stats("1.2.3.4", 30.0, 20.0, 0)),
        };
        assert_eq!(verdict(&m), Verdict::Insufficient);
        assert!(remote_unstable(&m).is_none());
    }

    #[test]
    fn too_few_replies_is_insufficient() {
        let m = Measurement {
            gateway: Some(PingStats::from_samples("g", &[Some(1.0); 10])),
            remote: None,
        };
        assert_eq!(verdict(&m), Verdict::Insufficient);
    }

    #[test]
    fn far_server_is_info_only() {
        let m = Measurement {
            gateway: Some(stats("g", 0.4, 0.1, 0)),
            remote: Some(stats("1.2.3.4", 120.0, 1.0, 0)),
        };
        let f = high_latency(&m).unwrap();
        assert_eq!(f.impact, Impact::Info);
        assert_eq!(verdict(&m), Verdict::Ok);
    }

    #[test]
    fn link_only_findings() {
        let weak = adapter(LinkKind::Wifi, Some(wifi(WifiBand::Ghz5, 30)));
        assert_eq!(ids(&analyze_link(Some(&weak))), vec!["wifi_weak"]);
        let vpn = adapter(LinkKind::Virtual, None);
        assert_eq!(ids(&analyze_link(Some(&vpn))), vec!["vpn_route"]);
        let mut slow = adapter(LinkKind::Ethernet, None);
        slow.link_mbps = Some(100);
        assert_eq!(ids(&analyze_link(Some(&slow))), vec!["slow_link"]);
        let band = adapter(LinkKind::Wifi, Some(wifi(WifiBand::Ghz2_4, 90)));
        let f = analyze_link(Some(&band));
        assert_eq!(f[0].impact, Impact::Info);
        assert!(analyze_link(None).is_empty());
    }

    #[test]
    fn every_text_is_localized() {
        let catalog = Catalog::ukrainian().unwrap();
        let wifi_bad = adapter(LinkKind::Wifi, Some(wifi(WifiBand::Ghz2_4, 30)));
        let mut slow = adapter(LinkKind::Ethernet, None);
        slow.link_mbps = Some(100);
        let cases = [
            (
                Measurement {
                    gateway: Some(stats("g", 3.0, 12.0, 20)),
                    remote: Some(stats("r", 120.0, 20.0, 20)),
                },
                wifi_bad.clone(),
            ),
            (
                Measurement {
                    gateway: Some(stats("g", 0.4, 0.1, 0)),
                    remote: Some(stats("r", 120.0, 20.0, 0)),
                },
                slow,
            ),
            (
                Measurement {
                    gateway: Some(stats("g", 0.4, 0.1, 0)),
                    remote: None,
                },
                adapter(LinkKind::Virtual, None),
            ),
            (
                Measurement {
                    gateway: None,
                    remote: None,
                },
                adapter(LinkKind::Ethernet, None),
            ),
            (
                Measurement {
                    gateway: Some(stats("g", 3.0, 12.0, 0)),
                    remote: None,
                },
                adapter(LinkKind::Ethernet, None),
            ),
        ];
        for (m, link) in cases {
            for f in analyze(&m, Some(&link)) {
                let mut texts = vec![f.title.clone(), f.detail.clone()];
                texts.extend(f.advice.clone());
                for e in &f.evidence {
                    texts.push(e.label.clone());
                    texts.push(e.value.clone());
                }
                for t in texts {
                    assert!(catalog.contains(&t.key), "{}", t.key);
                    assert!(!catalog.render(&t).contains('{'), "{}", t.key);
                }
            }
        }
    }
}
