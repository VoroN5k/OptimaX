//! Замір затримки: паралельні пінги до кількох цілей із проміжною статистикою.

use crate::model::PingStats;
use crate::NetworkError;
use std::net::{Ipv4Addr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Один ICMP echo. `Some(мс)` — відповідь прийшла, `None` — втрата / тайм-аут.
pub trait EchoApi: Send + Sync {
    /// Надсилає один запит і чекає відповіді не довше `timeout`.
    fn echo(&self, target: Ipv4Addr, timeout: Duration) -> Option<f64>;
}

/// Скасування заміру.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Новий.
    pub fn new() -> Self {
        Self::default()
    }

    /// Скасувати.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Чи скасовано.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Параметри заміру.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestOptions {
    /// Тривалість.
    pub duration: Duration,
    /// Інтервал між запитами до однієї цілі.
    pub interval: Duration,
    /// Тайм-аут відповіді (довший — втрата).
    pub timeout: Duration,
    /// Як часто віддавати проміжну статистику.
    pub progress_every: Duration,
}

impl TestOptions {
    /// 10 запитів на секунду до кожної цілі, тайм-аут 1 с.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration: duration.clamp(Duration::from_secs(5), Duration::from_secs(300)),
            interval: Duration::from_millis(100),
            timeout: Duration::from_secs(1),
            progress_every: Duration::from_millis(500),
        }
    }
}

/// IPv4-адреса з тексту: IP або ім'я вузла (тоді — один DNS-запит).
pub fn resolve(host: &str) -> Result<Ipv4Addr, NetworkError> {
    let host = host.trim();
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Ok(ip);
    }
    if host.is_empty() || host.len() > 253 {
        return Err(NetworkError::Resolve(host.into()));
    }
    (host, 0)
        .to_socket_addrs()
        .ok()
        .and_then(|mut it| {
            it.find_map(|a| match a.ip() {
                std::net::IpAddr::V4(v4) => Some(v4),
                std::net::IpAddr::V6(_) => None,
            })
        })
        .ok_or_else(|| NetworkError::Resolve(host.into()))
}

/// Пінгує всі цілі паралельно протягом `options.duration` (або до скасування).
/// `on_progress(статистика, минуло_секунд)` викликається періодично в потоці викликача.
/// Повертає фінальну статистику в порядку цілей.
pub fn run_test(
    api: &dyn EchoApi,
    targets: &[Ipv4Addr],
    options: TestOptions,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(&[PingStats], f64),
) -> Vec<PingStats> {
    let samples: Vec<Mutex<Vec<Option<f64>>>> =
        targets.iter().map(|_| Mutex::new(Vec::new())).collect();
    let started = Instant::now();
    let deadline = started + options.duration;
    let snapshot = |samples: &[Mutex<Vec<Option<f64>>>]| -> Vec<PingStats> {
        targets
            .iter()
            .zip(samples)
            .map(|(t, s)| {
                let data = s.lock().map(|v| v.clone()).unwrap_or_default();
                PingStats::from_samples(t.to_string(), &data)
            })
            .collect()
    };

    std::thread::scope(|scope| {
        for (target, store) in targets.iter().zip(&samples) {
            scope.spawn(move || {
                let mut next = Instant::now();
                while Instant::now() < deadline && !cancel.is_cancelled() {
                    let rtt = api.echo(*target, options.timeout);
                    if let Ok(mut v) = store.lock() {
                        v.push(rtt);
                    }
                    next += options.interval;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        // Відповідь ішла довше за інтервал — не надолужуємо пачкою запитів.
                        next = now;
                    }
                }
            });
        }
        let mut last = Instant::now();
        while Instant::now() < deadline && !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(50));
            if last.elapsed() >= options.progress_every {
                last = Instant::now();
                on_progress(&snapshot(&samples), started.elapsed().as_secs_f64());
            }
        }
    });
    snapshot(&samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// Імітація: роутер відповідає за 1 мс, «інтернет» — за 20 мс, кожен 10-й запит губиться.
    struct Mock {
        calls: AtomicU32,
    }

    impl EchoApi for Mock {
        fn echo(&self, target: Ipv4Addr, _timeout: Duration) -> Option<f64> {
            let n = self.calls.fetch_add(1, Ordering::Relaxed);
            if target.is_private() {
                Some(1.0)
            } else if n.is_multiple_of(10) {
                None
            } else {
                Some(20.0)
            }
        }
    }

    fn fast(duration_ms: u64) -> TestOptions {
        TestOptions {
            duration: Duration::from_millis(duration_ms),
            interval: Duration::from_millis(5),
            timeout: Duration::from_millis(10),
            progress_every: Duration::from_millis(50),
        }
    }

    #[test]
    fn pings_all_targets_in_parallel() {
        let api = Mock {
            calls: AtomicU32::new(0),
        };
        let gw: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let remote: Ipv4Addr = "8.8.8.8".parse().unwrap();
        let mut progress = 0;
        let out = run_test(
            &api,
            &[gw, remote],
            fast(300),
            &CancelToken::new(),
            &mut |s, _| {
                assert_eq!(s.len(), 2);
                progress += 1;
            },
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].target, "192.168.1.1");
        assert_eq!(out[0].median_ms, Some(1.0));
        assert_eq!(out[0].loss_pct, 0.0);
        assert!(out[0].sent >= 20, "надіслано {}", out[0].sent);
        assert_eq!(out[1].median_ms, Some(20.0));
        assert!(progress >= 2);
    }

    #[test]
    fn cancel_stops_early() {
        let api = Mock {
            calls: AtomicU32::new(0),
        };
        let cancel = CancelToken::new();
        cancel.cancel();
        let started = Instant::now();
        let out = run_test(
            &api,
            &["192.168.1.1".parse().unwrap()],
            fast(5_000),
            &cancel,
            &mut |_, _| {},
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(out[0].sent, 0);
    }

    #[test]
    fn resolve_parses_ip_and_rejects_garbage() {
        assert_eq!(resolve(" 1.1.1.1 ").unwrap(), Ipv4Addr::new(1, 1, 1, 1));
        assert!(resolve("").is_err());
        assert!(resolve(&"x".repeat(300)).is_err());
    }

    #[test]
    fn options_are_clamped() {
        assert_eq!(
            TestOptions::new(Duration::from_secs(1)).duration,
            Duration::from_secs(5)
        );
        assert_eq!(
            TestOptions::new(Duration::from_secs(9999)).duration,
            Duration::from_secs(300)
        );
    }
}
