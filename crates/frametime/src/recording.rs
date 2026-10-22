//! Одночасний запис кадрів (PresentMon) і системних метрик на спільній шкалі QPC.

use crate::csv::CsvFrameParser;
use crate::frame::{FrameRecord, FrameTimestamp};
use crate::presentmon::{BundledPresentMon, Target, MAX_CAPTURE};
use crate::FrametimeError;
use optimax_core::clock::{qpc_frequency, qpc_now, ticks_to_ms};
use optimax_metrics::{GpuApi, Sampler, SamplerConfig, Snapshot, SystemSource};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Скільки чекати PresentMon понад заданий час, перш ніж зупинити його примусово.
const GRACE: Duration = Duration::from_secs(15);
/// Скільки рядків stderr PresentMon зберігати для звіту.
const MAX_WARNINGS: usize = 20;

/// Прапорець скасування довгого запису (можна смикнути з іншого потоку чи UI).
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Новий токен.
    pub fn new() -> Self {
        Self::default()
    }

    /// Попросити зупинити запис.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Чи попросили зупинитись.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Параметри запису.
#[derive(Debug, Clone)]
pub struct RecordOptions {
    /// Яку гру записувати.
    pub target: Target,
    /// Тривалість (5 с – 10 хв).
    pub duration: Duration,
    /// Налаштування семплера метрик. Для кореляції з піками краще 200 мс.
    pub sampler: SamplerConfig,
}

impl RecordOptions {
    /// Типові параметри: 200 мс між знімками метрик, процеси раз на секунду.
    pub fn new(target: Target, duration: Duration) -> Self {
        Self {
            target,
            duration: duration.clamp(Duration::from_secs(5), MAX_CAPTURE),
            sampler: SamplerConfig {
                interval: Duration::from_millis(200),
                process_interval: Duration::from_secs(1),
                top_processes: 8,
            },
        }
    }
}

/// Результат запису.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Recording {
    /// Ціль.
    pub target: Target,
    /// Частота QPC.
    pub qpc_frequency: i64,
    /// QPC на момент старту запису — нуль шкали часу.
    pub start_qpc: i64,
    /// Усі кадри (усіх swap chain цілі).
    pub frames: Vec<FrameRecord>,
    /// Знімки метрик.
    pub snapshots: Vec<Snapshot>,
    /// Рядки CSV, які не вдалося розібрати.
    pub skipped_lines: usize,
    /// Повідомлення PresentMon зі stderr (попередження, помилки доступу).
    pub presentmon_messages: Vec<String>,
    /// Запис зупинено користувачем раніше часу.
    pub cancelled: bool,
}

impl Recording {
    /// Час кадру в мс від старту запису. Для формату без QPC — мс від старту PresentMon
    /// (зсув відносно метрик до ~100 мс, тому кореляція менш точна).
    pub fn frame_ms(&self, frame: &FrameRecord) -> f64 {
        match frame.time {
            FrameTimestamp::Qpc(t) => ticks_to_ms(t - self.start_qpc, self.qpc_frequency),
            FrameTimestamp::Ms(ms) => ms,
        }
    }

    /// Час знімка метрик у мс від старту запису.
    pub fn snapshot_ms(&self, s: &Snapshot) -> f64 {
        ticks_to_ms(s.qpc - self.start_qpc, self.qpc_frequency)
    }
}

/// Записує кадри й метрики. Блокує потік на час запису — викликати не з UI-потоку.
pub fn record(
    presentmon: &BundledPresentMon,
    options: &RecordOptions,
    system: Box<dyn SystemSource>,
    gpu: Option<Box<dyn GpuApi>>,
    cancel: &CancelToken,
) -> Result<Recording, FrametimeError> {
    options.target.validate()?;
    let start_qpc = qpc_now();
    let (sampler, snapshots_rx) = Sampler::spawn(options.sampler, system, gpu)?;
    let mut child = presentmon.spawn_capture(&options.target, options.duration)?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let frames_thread = std::thread::Builder::new()
        .name("optimax-presentmon-csv".into())
        .spawn(move || -> (Vec<FrameRecord>, usize) {
            let Some(out) = stdout else {
                return (Vec::new(), 0);
            };
            let mut lines = BufReader::new(out).lines();
            let parser = loop {
                match lines.next() {
                    Some(Ok(l)) => match CsvFrameParser::from_header(&l) {
                        Ok(p) => break p,
                        // До заголовка PresentMon може писати службові рядки.
                        Err(_) => continue,
                    },
                    _ => return (Vec::new(), 0),
                }
            };
            let mut frames = Vec::new();
            let mut skipped = 0;
            for line in lines.map_while(Result::ok) {
                match parser.parse_line(&line) {
                    Ok(Some(f)) => frames.push(f),
                    Ok(None) => {}
                    Err(_) => skipped += 1,
                }
            }
            (frames, skipped)
        })?;
    let messages_thread = std::thread::Builder::new()
        .name("optimax-presentmon-err".into())
        .spawn(move || -> Vec<String> {
            let Some(err) = stderr else {
                return Vec::new();
            };
            BufReader::new(err)
                .lines()
                .map_while(Result::ok)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .take(MAX_WARNINGS)
                .collect()
        })?;

    let deadline = Instant::now() + options.duration + GRACE;
    let mut snapshots = Vec::new();
    let mut cancelled = false;
    loop {
        if let Ok(s) = snapshots_rx.recv_timeout(Duration::from_millis(100)) {
            snapshots.push(s);
        }
        if child.try_wait()?.is_some() {
            break;
        }
        if cancel.is_cancelled() || Instant::now() > deadline {
            cancelled = cancel.is_cancelled();
            if !cancelled {
                tracing::warn!("PresentMon не завершився вчасно — зупиняємо примусово");
            }
            if let Err(e) = presentmon.stop_session() {
                tracing::warn!("не вдалося зупинити ETW-сесію: {e}");
            }
            if child.try_wait()?.is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
            break;
        }
    }
    sampler.stop();
    snapshots.extend(snapshots_rx.try_iter());

    let (frames, skipped_lines) = frames_thread.join().unwrap_or_default();
    let presentmon_messages = messages_thread.join().unwrap_or_default();
    if frames.is_empty() {
        return Err(FrametimeError::NoFrames(presentmon_messages.join(" | ")));
    }
    Ok(Recording {
        target: options.target.clone(),
        qpc_frequency: qpc_frequency(),
        start_qpc,
        frames,
        snapshots,
        skipped_lines,
        presentmon_messages,
        cancelled,
    })
}
