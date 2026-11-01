//! Живі дані для оверлею в грі: FPS, 1% low і час кадру гри у фокусі, CPU і GPU.
//!
//! [`LiveMonitor`] тримає PresentMon по всіх процесах в окремій ETW-сесії
//! ([`crate::presentmon::LIVE_SESSION_NAME`] — не заважає запису й стеженню) і легкий семплер
//! метрик (500 мс, без переліку процесів). У рахунок іде лише процес, чиє вікно на передньому
//! плані; змінився фокус — вікно статистики починається заново. Нічого в системі не змінює.

use crate::csv::CsvFrameParser;
use crate::frame::{FrameRecord, FrameTimestamp};
use crate::presentmon::{BundledPresentMon, MAX_CAPTURE};
use crate::stats::frame_stats;
use crate::FrametimeError;
use optimax_core::clock::qpc_frequency;
use optimax_metrics::{GpuApi, Sampler, SamplerConfig, Snapshot, SystemSource};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// За скільки останніх секунд рахується FPS і час кадру (швидка реакція).
pub const FPS_WINDOW: Duration = Duration::from_secs(2);
/// За скільки — 1% low (коротше вікно дає надто стрибучу цифру).
pub const LOW_WINDOW: Duration = Duration::from_secs(10);
/// Найменше кадрів у вікні 1% low, щоб його показувати.
pub const LOW_MIN_FRAMES: usize = 100;
/// Кадри не надходять довше — гра не малює (меню на паузі, Alt+Tab): даних немає.
pub const STALE: Duration = Duration::from_secs(2);
/// Такт оновлення.
const TICK: Duration = Duration::from_millis(250);
/// PresentMon, що завершився раніше за це після старту, вважається збоєм запуску.
const QUICK_EXIT: Duration = Duration::from_secs(5);
/// Скільки збоїв запуску поспіль терпіти.
const MAX_QUICK_EXITS: u32 = 3;

/// Цифри для оверлею.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveStats {
    /// Ім'я процесу гри.
    pub app: String,
    /// Середній FPS за [`FPS_WINDOW`].
    pub fps: f64,
    /// Середній час кадру за [`FPS_WINDOW`], мс.
    pub frametime_ms: f64,
    /// 1% low за [`LOW_WINDOW`] (`None` — ще замало кадрів).
    pub low_1pct_fps: Option<f64>,
    /// CPU, % усього процесора.
    pub cpu_percent: Option<f32>,
    /// Найзавантаженіше логічне ядро, %.
    pub cpu_core_percent: Option<f32>,
    /// Завантаження GPU, % (`None` — немає джерела, напр. не NVIDIA).
    pub gpu_percent: Option<u32>,
}

/// Кадрові цифри гри у фокусі.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameNumbers {
    /// FPS за [`FPS_WINDOW`].
    pub fps: f64,
    /// Час кадру за [`FPS_WINDOW`], мс.
    pub frametime_ms: f64,
    /// 1% low за [`LOW_WINDOW`].
    pub low_1pct_fps: Option<f64>,
}

/// Вікно кадрів одного процесу (чиста структура — логіку перевіряють тести).
#[derive(Debug)]
pub struct FrameWindow {
    freq: i64,
    pid: Option<u32>,
    app: String,
    /// (QPC кадру, мс між показами).
    frames: VecDeque<(i64, f64)>,
}

impl FrameWindow {
    /// Порожнє вікно для частоти QPC `freq`.
    pub fn new(freq: i64) -> Self {
        Self {
            freq,
            pid: None,
            app: String::new(),
            frames: VecDeque::new(),
        }
    }

    /// Процес у фокусі. Інший, ніж був, — статистика починається заново.
    pub fn focus(&mut self, pid: Option<u32>) {
        if self.pid != pid {
            self.pid = pid;
            self.frames.clear();
            self.app.clear();
        }
    }

    /// Додає кадр; кадри інших процесів ігноруються. Повертає, чи кадр узято.
    pub fn push(&mut self, f: &FrameRecord) -> bool {
        let (Some(pid), FrameTimestamp::Qpc(t)) = (self.pid, f.time) else {
            return false;
        };
        if f.process_id != pid {
            return false;
        }
        if self.app.is_empty() {
            self.app.clone_from(&f.application);
        }
        self.frames.push_back((t, f.ms_between_presents));
        let horizon = t - ticks(LOW_WINDOW, self.freq);
        while self.frames.front().is_some_and(|(q, _)| *q < horizon) {
            self.frames.pop_front();
        }
        true
    }

    /// Ім'я процесу (з кадрів).
    pub fn app(&self) -> &str {
        &self.app
    }

    /// Цифри від найновішого кадру назад (кадри PresentMon надходять із затримкою ~1 с, тож
    /// рахувати від «зараз» не можна — вікно було б напівпорожнім).
    pub fn numbers(&self) -> Option<FrameNumbers> {
        let newest = self.frames.back()?.0;
        let from = newest - ticks(FPS_WINDOW, self.freq);
        let recent: Vec<f64> = self
            .frames
            .iter()
            .filter(|(q, _)| *q >= from)
            .map(|(_, ms)| *ms)
            .collect();
        let fast = frame_stats(&recent)?;
        let all: Vec<f64> = self.frames.iter().map(|(_, ms)| *ms).collect();
        let low = (all.len() >= LOW_MIN_FRAMES)
            .then(|| frame_stats(&all).map(|s| s.low_1pct_fps))
            .flatten();
        Some(FrameNumbers {
            fps: fast.avg_fps,
            frametime_ms: 1000.0 / fast.avg_fps.max(f64::MIN_POSITIVE),
            low_1pct_fps: low,
        })
    }
}

fn ticks(d: Duration, freq: i64) -> i64 {
    i64::try_from(d.as_micros())
        .unwrap_or(i64::MAX / 2)
        .saturating_mul(freq)
        / 1_000_000
}

/// Живі дані оверлею. Зупиняється [`LiveMonitor::stop`] або при знищенні.
pub struct LiveMonitor {
    latest: Arc<Mutex<Option<LiveStats>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for LiveMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveMonitor")
    }
}

impl LiveMonitor {
    /// Запускає PresentMon (живі дані, окрема сесія) і семплер.
    pub fn start(
        presentmon: &BundledPresentMon,
        system: Box<dyn SystemSource>,
        gpu: Option<Box<dyn GpuApi>>,
    ) -> Result<Self, FrametimeError> {
        let config = SamplerConfig {
            interval: Duration::from_millis(500),
            // Перелік процесів не потрібен: лише раз на старті (найдорожча частина семплера).
            process_interval: Duration::from_secs(3600),
            top_processes: 0,
        };
        let (sampler, snapshots) = Sampler::spawn(config, system, gpu)?;
        let pm = Live::spawn(presentmon)?;
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let ctx = Worker {
            presentmon: presentmon.clone(),
            sampler: Some(sampler),
            snapshots,
            pm: Some(pm),
            latest: latest.clone(),
            stop: stop.clone(),
        };
        let handle = std::thread::Builder::new()
            .name("optimax-live".into())
            .spawn(move || ctx.run())?;
        Ok(Self {
            latest,
            stop,
            handle: Some(handle),
        })
    }

    /// Останні цифри (`None` — гра у фокусі не малює кадрів або фокус не на грі).
    pub fn latest(&self) -> Option<LiveStats> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Чи працює ще збір (PresentMon не відмовив).
    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Зупиняє збір і чекає звільнення ресурсів.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for LiveMonitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Процес PresentMon (живі дані) і потік читання CSV.
struct Live {
    child: Child,
    started: Instant,
    frames: Receiver<FrameRecord>,
}

impl Live {
    fn spawn(presentmon: &BundledPresentMon) -> Result<Self, FrametimeError> {
        let mut child = presentmon.spawn_live(MAX_CAPTURE)?;
        let stdout = child.stdout.take();
        // stderr не читаємо, але й не лишаємо переповнюватись.
        drop(child.stderr.take());
        let (tx, frames) = mpsc::channel();
        std::thread::Builder::new()
            .name("optimax-live-csv".into())
            .spawn(move || {
                let Some(out) = stdout else { return };
                let mut lines = BufReader::new(out).lines();
                let parser = loop {
                    match lines.next() {
                        Some(Ok(l)) => match CsvFrameParser::from_header(&l) {
                            Ok(p) => break p,
                            Err(_) => continue,
                        },
                        _ => return,
                    }
                };
                for line in lines.map_while(Result::ok) {
                    if let Ok(Some(f)) = parser.parse_line(&line) {
                        if tx.send(f).is_err() {
                            return;
                        }
                    }
                }
            })?;
        Ok(Self {
            child,
            started: Instant::now(),
            frames,
        })
    }

    fn shutdown(mut self, presentmon: &BundledPresentMon) {
        if matches!(self.child.try_wait(), Ok(None)) {
            if let Err(e) = presentmon.stop_live_session() {
                tracing::warn!("не вдалося зупинити ETW-сесію оверлею: {e}");
            }
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
            }
        }
        let _ = self.child.wait();
    }
}

struct Worker {
    presentmon: BundledPresentMon,
    sampler: Option<Sampler>,
    snapshots: Receiver<Snapshot>,
    pm: Option<Live>,
    latest: Arc<Mutex<Option<LiveStats>>>,
    stop: Arc<AtomicBool>,
}

impl Worker {
    fn run(mut self) {
        self.work();
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if let Some(pm) = self.pm.take() {
            pm.shutdown(&self.presentmon);
        }
        if let Some(s) = self.sampler.take() {
            s.stop();
        }
    }

    fn work(&mut self) {
        let own_pid = std::process::id();
        let mut window = FrameWindow::new(qpc_frequency());
        let mut snapshot: Option<Snapshot> = None;
        let mut last_frame: Option<Instant> = None;
        let mut quick_exits = 0u32;
        while !self.stop.load(Ordering::SeqCst) {
            if let Ok(s) = self.snapshots.recv_timeout(TICK) {
                snapshot = Some(s);
            }
            snapshot = self.snapshots.try_iter().last().or(snapshot);
            let Some(pm) = self.pm.as_mut() else { return };
            if !matches!(pm.child.try_wait(), Ok(None)) {
                let quick = pm.started.elapsed() < QUICK_EXIT;
                if let Some(old) = self.pm.take() {
                    old.shutdown(&self.presentmon);
                }
                quick_exits = if quick { quick_exits + 1 } else { 0 };
                if quick_exits >= MAX_QUICK_EXITS {
                    tracing::error!("оверлей: PresentMon не запускається");
                    return;
                }
                match Live::spawn(&self.presentmon) {
                    Ok(p) => self.pm = Some(p),
                    Err(e) => {
                        tracing::error!("оверлей: PresentMon не запустився: {e}");
                        return;
                    }
                }
                continue;
            }
            let fg = optimax_core::input::foreground_process_id().filter(|p| *p != own_pid);
            window.focus(fg);
            for f in pm.frames.try_iter() {
                if window.push(&f) {
                    last_frame = Some(Instant::now());
                }
            }
            let fresh = fg.is_some() && last_frame.is_some_and(|t| t.elapsed() < STALE);
            let stats = window.numbers().filter(|_| fresh).map(|n| LiveStats {
                app: window.app().to_string(),
                fps: n.fps,
                frametime_ms: n.frametime_ms,
                low_1pct_fps: n.low_1pct_fps,
                cpu_percent: snapshot.as_ref().map(|s| s.cpu.total_percent),
                cpu_core_percent: snapshot
                    .as_ref()
                    .and_then(|s| s.cpu.busiest_core())
                    .map(|(_, p)| p),
                gpu_percent: snapshot
                    .as_ref()
                    .and_then(|s| s.gpus.iter().filter_map(|g| g.utilization_percent).max()),
            });
            *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = stats;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1000 тиків = 1 с.
    const F: i64 = 1000;

    fn frame(pid: u32, qpc: i64, ms: f64) -> FrameRecord {
        FrameRecord {
            application: format!("g{pid}.exe"),
            process_id: pid,
            swap_chain: "0x1".into(),
            present_mode: String::new(),
            time: FrameTimestamp::Qpc(qpc),
            ms_between_presents: ms,
            ms_between_display_change: None,
            ms_cpu_busy: None,
            ms_gpu_busy: None,
            sync_interval: None,
            allows_tearing: None,
        }
    }

    #[test]
    fn counts_only_focused_process() {
        let mut w = FrameWindow::new(F);
        assert!(!w.push(&frame(1, 0, 10.0)), "без фокусу нічого не береться");
        w.focus(Some(1));
        for i in 0..200 {
            assert!(w.push(&frame(1, i * 10, 10.0)));
            assert!(!w.push(&frame(2, i * 10, 5.0)));
        }
        assert_eq!(w.app(), "g1.exe");
        let n = w.numbers().unwrap();
        assert!((n.fps - 100.0).abs() < 1e-9);
        assert!((n.frametime_ms - 10.0).abs() < 1e-9);
        assert!(n.low_1pct_fps.is_some());
        // Фокус на іншому процесі — статистика заново.
        w.focus(Some(2));
        assert!(w.numbers().is_none());
        assert_eq!(w.app(), "");
    }

    #[test]
    fn fps_uses_recent_two_seconds_and_low_uses_ten() {
        let mut w = FrameWindow::new(F);
        w.focus(Some(1));
        // 8 с по 10 мс (100 FPS), потім 2 с по 20 мс (50 FPS).
        let mut t = 0;
        while t < 8 * F {
            w.push(&frame(1, t, 10.0));
            t += 10;
        }
        while t < 10 * F {
            w.push(&frame(1, t, 20.0));
            t += 20;
        }
        let n = w.numbers().unwrap();
        assert!((n.fps - 50.0).abs() < 1.0, "{}", n.fps);
        // 1% low — найгірші кадри за 10 с, тобто 20 мс.
        assert!((n.low_1pct_fps.unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn old_frames_leave_the_window_and_low_needs_enough_frames() {
        let mut w = FrameWindow::new(F);
        w.focus(Some(1));
        for i in 0..50 {
            w.push(&frame(1, i * 20, 20.0));
        }
        assert!(
            w.numbers().unwrap().low_1pct_fps.is_none(),
            "50 кадрів — замало"
        );
        // Кадр через 30 с: усе старше за 10 с відкидається.
        w.push(&frame(1, 30 * F, 20.0));
        assert_eq!(w.frames.len(), 1);
    }
}
