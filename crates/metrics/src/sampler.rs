//! Фоновий потік, який періодично збирає [`Snapshot`].

use crate::gpu::GpuApi;
use crate::snapshot::{ProcessSample, Snapshot};
use crate::source::SystemSource;
use crate::MetricsError;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Найменший дозволений інтервал.
pub const MIN_INTERVAL: Duration = Duration::from_millis(100);
/// Найбільший дозволений інтервал.
pub const MAX_INTERVAL: Duration = Duration::from_millis(500);
/// Скільки знімків тримати в каналі, поки споживач не встигає (старіші не накопичуються).
const CHANNEL_CAPACITY: usize = 64;

/// Налаштування семплера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerConfig {
    /// Інтервал між знімками, обрізається до 100–500 мс.
    /// Нижче 200 мс точність завантаження CPU на Windows падає (обмеження лічильників).
    pub interval: Duration,
    /// Як часто оновлювати список процесів (найдорожча частина), не частіше за `interval`.
    pub process_interval: Duration,
    /// Скільки процесів тримати в топі.
    pub top_processes: usize,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(250),
            process_interval: Duration::from_secs(1),
            top_processes: 5,
        }
    }
}

impl SamplerConfig {
    /// Інтервал у дозволених межах.
    pub fn clamped_interval(&self) -> Duration {
        self.interval.clamp(MIN_INTERVAL, MAX_INTERVAL)
    }

    /// Кожен який тік оновлювати процеси (≥ 1).
    pub fn process_every_ticks(&self) -> u64 {
        let tick = self.clamped_interval().as_millis().max(1);
        let ticks = self.process_interval.as_millis().div_ceil(tick);
        u64::try_from(ticks).unwrap_or(u64::MAX).max(1)
    }
}

/// Запущений семплер. Зупиняється через [`Sampler::stop`] або при знищенні.
pub struct Sampler {
    stop_tx: Option<SyncSender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl Sampler {
    /// Запускає збір в окремому потоці. Знімки приходять у повернений канал;
    /// якщо споживач не встигає, нові знімки відкидаються (без блокування й росту пам'яті).
    pub fn spawn(
        config: SamplerConfig,
        mut system: Box<dyn SystemSource>,
        mut gpu: Option<Box<dyn GpuApi>>,
    ) -> Result<(Self, Receiver<Snapshot>), MetricsError> {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (stop_tx, stop_rx) = mpsc::sync_channel::<()>(1);
        let interval = config.clamped_interval();
        let process_every = config.process_every_ticks();

        let handle = std::thread::Builder::new()
            .name("optimax-metrics".into())
            .spawn(move || {
                let start = Instant::now();
                let mut last = start;
                let mut next = start + interval;
                let mut tick: u64 = 0;
                let mut top: Vec<ProcessSample> = Vec::new();
                loop {
                    let wait = next.saturating_duration_since(Instant::now());
                    match stop_rx.recv_timeout(wait) {
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    }
                    let now = Instant::now();
                    let elapsed = now.duration_since(last);
                    last = now;
                    // Якщо потік відстав (сон системи), не наздоганяємо пачкою знімків.
                    next = (next + interval).max(now);

                    if tick.is_multiple_of(process_every) {
                        top = system.top_processes(config.top_processes);
                    }
                    tick = tick.wrapping_add(1);

                    let snapshot = Snapshot {
                        elapsed_ms: u64::try_from(now.duration_since(start).as_millis())
                            .unwrap_or(u64::MAX),
                        cpu: system.cpu(),
                        memory: system.memory(),
                        disk: system.disk(elapsed),
                        gpus: gpu.as_mut().map(|g| g.sample()).unwrap_or_default(),
                        top_processes: top.clone(),
                    };
                    match tx.try_send(snapshot) {
                        Ok(()) | Err(TrySendError::Full(_)) => {}
                        Err(TrySendError::Disconnected(_)) => break,
                    }
                }
                tracing::debug!("семплер метрик зупинено");
            })?;

        Ok((
            Self {
                stop_tx: Some(stop_tx),
                handle: Some(handle),
            },
            rx,
        ))
    }

    /// Зупиняє потік і чекає його завершення.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.try_send(());
        }
        if let Some(h) = self.handle.take() {
            if h.join().is_err() {
                tracing::error!("потік метрик завершився з панікою");
            }
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{CpuSnapshot, DiskSnapshot, GpuSnapshot, MemorySnapshot};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct FakeSystem {
        process_calls: Arc<AtomicU32>,
    }

    impl SystemSource for FakeSystem {
        fn cpu(&mut self) -> CpuSnapshot {
            CpuSnapshot {
                total_percent: 10.0,
                per_core_percent: vec![10.0, 96.0],
                per_core_mhz: vec![4000, 4000],
            }
        }
        fn memory(&mut self) -> MemorySnapshot {
            MemorySnapshot {
                total_bytes: 32,
                used_bytes: 16,
                available_bytes: 16,
            }
        }
        fn disk(&mut self, _elapsed: Duration) -> DiskSnapshot {
            DiskSnapshot::default()
        }
        fn top_processes(&mut self, _n: usize) -> Vec<ProcessSample> {
            self.process_calls.fetch_add(1, Ordering::SeqCst);
            vec![]
        }
    }

    struct FakeGpu;
    impl GpuApi for FakeGpu {
        fn sample(&mut self) -> Vec<GpuSnapshot> {
            vec![GpuSnapshot {
                utilization_percent: Some(99),
                ..GpuSnapshot::default()
            }]
        }
    }

    #[test]
    fn interval_is_clamped_and_process_ticks_computed() {
        let mut c = SamplerConfig {
            interval: Duration::from_millis(10),
            ..SamplerConfig::default()
        };
        assert_eq!(c.clamped_interval(), MIN_INTERVAL);
        assert_eq!(c.process_every_ticks(), 10);
        c.interval = Duration::from_secs(5);
        assert_eq!(c.clamped_interval(), MAX_INTERVAL);
        assert_eq!(c.process_every_ticks(), 2);
        c.process_interval = Duration::ZERO;
        assert_eq!(c.process_every_ticks(), 1);
    }

    #[test]
    fn produces_snapshots_and_stops() {
        let calls = Arc::new(AtomicU32::new(0));
        let config = SamplerConfig {
            interval: Duration::from_millis(100),
            process_interval: Duration::from_millis(300),
            top_processes: 3,
        };
        let (sampler, rx) = Sampler::spawn(
            config,
            Box::new(FakeSystem {
                process_calls: calls.clone(),
            }),
            Some(Box::new(FakeGpu)),
        )
        .unwrap();
        let snaps: Vec<Snapshot> = (0..4)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        sampler.stop();
        assert!(snaps.windows(2).all(|w| w[0].elapsed_ms < w[1].elapsed_ms));
        assert_eq!(snaps[0].cpu.busiest_core(), Some((1, 96.0)));
        assert_eq!(snaps[0].gpus[0].utilization_percent, Some(99));
        // 4 тіки при оновленні процесів кожні 3 тіки → 2 виклики (тіки 0 і 3).
        assert!((2..=3).contains(&calls.load(Ordering::SeqCst)));
    }
}
