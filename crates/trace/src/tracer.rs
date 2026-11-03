//! Трасування потоків гри на час запису: системна сесія (потоки, перемикання, готовність) +
//! DXGI (хто викликає Present), обробка по ходу запису, підсумок — [`ThreadTrace`].

use crate::threads::{ThreadCollector, ThreadTrace, THREAD_CLASS};
use crate::{provider_guid, ProviderSpec, Session, SessionKind, TraceError};
use optimax_core::clock::{qpc_frequency, qpc_now};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

/// Прапорці ядра: потоки (`THREAD`), перемикання (`CSWITCH`), готовність (`DISPATCHER`).
const KERNEL_FLAGS: u32 = 0x2 | 0x10 | 0x800;
/// DXGI: ключове слово подій Present (маска з заміру на RE9 2026-11-03).
const DXGI_PRESENT_KEYWORD: u64 = 0x2;
/// DXGI `Present / Start`.
const DXGI_PRESENT_START: u16 = 42;
/// Як часто обробляти накопичене.
const DRAIN_EVERY: Duration = Duration::from_millis(500);
/// Наскільки відставати від найсвіжішої події: буфери процесорів доходять із затримкою
/// (сесія скидає буфери раз на секунду).
const LAG_SECS: f64 = 2.5;

struct Shared {
    threads: ThreadCollector,
    presents: HashMap<u32, u64>,
}

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Запущене трасування потоків гри.
pub struct ThreadTracer {
    kernel: Option<Session>,
    dxgi: Option<Session>,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    drainer: Option<JoinHandle<()>>,
    freq: i64,
}

impl std::fmt::Debug for ThreadTracer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThreadTracer")
    }
}

impl ThreadTracer {
    /// Починає трасування потоків процесу `game_pid`; кошики — від `start_qpc`.
    /// Без DXGI працює далі (рендер-потік тоді невідомий).
    pub fn start(game_pid: u32, start_qpc: i64) -> Result<Self, TraceError> {
        let freq = qpc_frequency();
        let shared = Arc::new(Mutex::new(Shared {
            threads: ThreadCollector::new(game_pid, start_qpc, freq),
            presents: HashMap::new(),
        }));
        let kernel = {
            let s = shared.clone();
            Session::start(
                "OptimaX Thread Trace",
                &SessionKind::Kernel {
                    flags: KERNEL_FLAGS,
                },
                move |r| {
                    let k = r.key();
                    if k.provider == THREAD_CLASS {
                        lock(&s).threads.on_thread_event(
                            k.opcode,
                            r.timestamp(),
                            r.cpu(),
                            r.data(),
                        );
                    }
                },
            )?
        };
        let dxgi = provider_guid("Microsoft-Windows-DXGI").and_then(|guid| {
            let s = shared.clone();
            Session::start(
                "OptimaX Present Trace",
                &SessionKind::Providers(vec![ProviderSpec {
                    guid,
                    level: 4,
                    keywords: DXGI_PRESENT_KEYWORD,
                }]),
                move |r| {
                    if r.key().id == DXGI_PRESENT_START && r.pid() == game_pid {
                        *lock(&s).presents.entry(r.tid()).or_default() += 1;
                    }
                },
            )
            .map_err(|e| tracing::warn!("трасування DXGI недоступне: {e}"))
            .ok()
        });
        let stop = Arc::new(AtomicBool::new(false));
        let lag = (LAG_SECS * freq as f64) as i64;
        let drainer = {
            let shared = shared.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("optimax-thread-trace".into())
                .spawn(move || {
                    let mut picked = false;
                    while !stop.load(Ordering::SeqCst) {
                        std::thread::sleep(DRAIN_EVERY);
                        let watermark = qpc_now() - lag;
                        let mut g = lock(&shared);
                        g.threads.drain(watermark);
                        if !picked {
                            // Рендер — потік, що найчастіше викликає Present.
                            let render: Vec<u32> = g
                                .presents
                                .iter()
                                .max_by_key(|(_, n)| **n)
                                .map(|(t, _)| *t)
                                .into_iter()
                                .collect();
                            picked = g.threads.pick_detail(&render);
                        }
                    }
                })
                .map_err(|_| TraceError::Api {
                    call: "spawn",
                    code: 0,
                })?
        };
        Ok(Self {
            kernel: Some(kernel),
            dxgi,
            shared,
            stop,
            drainer: Some(drainer),
            freq,
        })
    }

    /// Зупиняє трасування й повертає підсумок потоків гри; точні проміжки головних потоків
    /// лишаються лише у вікнах `keep` (тики QPC — навколо ривків).
    pub fn stop(mut self, keep: &[(i64, i64)]) -> ThreadTrace {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.drainer.take() {
            let _ = h.join();
        }
        let lost =
            self.kernel.take().map_or(0, Session::stop) + self.dxgi.take().map_or(0, Session::stop);
        let mut g = lock(&self.shared);
        let threads = std::mem::replace(&mut g.threads, ThreadCollector::new(0, 0, self.freq));
        let presents = std::mem::take(&mut g.presents);
        drop(g);
        threads.finish(&presents, lost, self.freq, keep)
    }
}

impl Drop for ThreadTracer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.drainer.take() {
            let _ = h.join();
        }
    }
}
