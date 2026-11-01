//! «Щойно лагнуло»: кільцевий буфер останніх секунд гри.
//!
//! [`Watcher`] тримає запущеними PresentMon, семплер метрик і трасування DPC (кільцевий режим),
//! але зберігає лише останні [`KEEP`] секунд. Натискання `Ctrl+Shift+F11` (або
//! [`Watcher::capture_now`]) фіксує момент; через [`DRAIN`] (кадри PresentMon приходять із
//! затримкою) з буфера вирізається вікно `[момент − window, момент]` і віддається як звичайний
//! [`Recording`] — далі його аналізує той самий `diagnose`.
//!
//! Зараховується лише час, коли гра на передньому плані й малює кадри, після [`RESUME_WARMUP`]
//! (як у звичайному записі: перші секунди після Alt+Tab — не гра). Нічого в системі не змінює.

use crate::csv::CsvFrameParser;
use crate::frame::{FrameRecord, FrameTimestamp};
use crate::gate::{EndReason, PRESENTING_WINDOW, RESUME_WARMUP};
use crate::presentmon::{BundledPresentMon, Target, MAX_CAPTURE};
use crate::recording::Recording;
use crate::FrametimeError;
use optimax_core::clock::{qpc_frequency, qpc_now, ticks_to_ms};
use optimax_core::toast::{Toaster, Tone};
use optimax_metrics::{GpuApi, Sampler, SamplerConfig, Snapshot, SystemSource};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Скільки секунд гри зберігає знімок за замовчуванням.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(30);
/// Скільки тримати в буфері: вікно + запас на затримку кадрів і неточність часу.
pub const KEEP: Duration = Duration::from_secs(40);
/// Скільки чекати після натискання, поки PresentMon допише кадри до цього моменту.
pub const DRAIN: Duration = Duration::from_millis(2500);
/// Найменше зарахованої гри у вікні, з якого ще є сенс робити висновки.
pub const MIN_GAME: Duration = Duration::from_secs(5);
/// Такт основного циклу.
const TICK: Duration = Duration::from_millis(100);
/// Як часто перевіряти, чи процес гри ще живий.
const PROCESS_CHECK: Duration = Duration::from_millis(500);
/// Як часто повідомляти стан.
const STATUS_EVERY: Duration = Duration::from_secs(1);
/// Скільки рядків stderr PresentMon зберігати.
const MAX_MESSAGES: usize = 20;
/// Кадрів у буфері щонайбільше (захист пам'яті: 40 с × 1000 FPS з запасом).
const MAX_FRAMES: usize = 60_000;
/// PresentMon, що завершився раніше за це після старту, вважається збоєм запуску.
const QUICK_EXIT: Duration = Duration::from_secs(5);
/// Скільки збоїв запуску PresentMon поспіль терпіти.
const MAX_QUICK_EXITS: u32 = 3;
/// Скільки показувати повідомлення поверх гри.
const NOTICE: Duration = Duration::from_millis(2000);

/// Параметри стеження.
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// Яку гру.
    pub target: Target,
    /// Скільки останніх секунд зберігає знімок (5 с – [`KEEP`] − 5 с).
    pub window: Duration,
    /// Налаштування семплера метрик.
    pub sampler: SamplerConfig,
    /// Звуковий сигнал при збереженні.
    pub sound: bool,
    /// Повідомлення поверх гри; `None` — без них.
    pub notices: Option<WatchNotices>,
}

impl WatchOptions {
    /// Стеження за грою з вікном 30 с і семплером як у звичайному записі (200 мс).
    pub fn new(target: Target) -> Self {
        Self {
            target,
            window: DEFAULT_WINDOW,
            sampler: SamplerConfig {
                interval: Duration::from_millis(200),
                process_interval: Duration::from_secs(1),
                top_processes: 8,
            },
            sound: false,
            notices: None,
        }
    }

    fn window(&self) -> Duration {
        self.window
            .clamp(MIN_GAME, KEEP.saturating_sub(Duration::from_secs(5)))
    }
}

/// Тексти повідомлень поверх гри.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchNotices {
    /// Після натискання: «зберігаю останні 30 с».
    pub saving: String,
    /// Знімок не вдався (замало гри у вікні).
    pub too_short: String,
}

/// Стан для інтерфейсу.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WatchStatus {
    /// Процес гри запущено.
    pub game_running: bool,
    /// Зараз іде зарахована гра (у фокусі, малює кадри, після прогріву).
    pub in_game: bool,
    /// Скільки секунд зарахованої гри зараз у буфері (не більше вікна).
    pub buffered_s: f64,
    /// Трасування драйверів (DPC) працює.
    pub dpc: bool,
}

/// Чому знімок не вдався.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CaptureFailure {
    /// У вікні замало часу, коли гра була у фокусі й малювала кадри.
    NotEnoughGame,
    /// У вікні немає кадрів гри.
    NoFrames,
}

/// Чому стеження завершилось.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum WatchEnd {
    /// Зупинено користувачем.
    Stopped,
    /// Гру закрито.
    GameExited,
    /// PresentMon не запускається (текст — його повідомлення).
    PresentMonFailed(String),
}

/// Подія стеження.
#[derive(Debug)]
pub enum WatchEvent {
    /// Стан (раз на секунду).
    Status(WatchStatus),
    /// Натиснуто гарячу клавішу (або кнопку): знімок буде за [`DRAIN`].
    Pressed,
    /// Знімок останніх секунд.
    Captured {
        /// Запис вікна (зараховані відрізки — лише гра у фокусі).
        recording: Box<Recording>,
        /// Unix-час натискання, мс.
        at_unix_ms: Option<i64>,
    },
    /// Знімок не вдався.
    CaptureFailed {
        /// Причина.
        reason: CaptureFailure,
        /// Unix-час натискання, мс.
        at_unix_ms: Option<i64>,
    },
    /// Стеження завершилось (подій більше не буде).
    Ended(WatchEnd),
}

/// Кільцевий буфер: кадри, знімки метрик і відрізки зарахованої гри (усе — у тиках QPC).
/// Чиста структура без системних викликів — логіку перевіряють тести.
#[derive(Debug)]
pub struct Ring {
    freq: i64,
    keep_ticks: i64,
    warmup_ticks: i64,
    frames: VecDeque<FrameRecord>,
    snapshots: VecDeque<Snapshot>,
    segments: VecDeque<(i64, i64)>,
    /// Початок поточного відрізка гри у фокусі (ще до прогріву).
    focus_since: Option<i64>,
    /// Початок поточного зарахованого відрізка.
    open: Option<i64>,
}

impl Ring {
    /// Порожній буфер на `keep` для частоти QPC `freq`.
    pub fn new(freq: i64, keep: Duration) -> Self {
        Self {
            freq,
            keep_ticks: dur_ticks(keep, freq),
            warmup_ticks: dur_ticks(RESUME_WARMUP, freq),
            frames: VecDeque::new(),
            snapshots: VecDeque::new(),
            segments: VecDeque::new(),
            focus_since: None,
            open: None,
        }
    }

    /// Додає кадр (кадри без часу QPC не зберігаються — їх не зіставити з метриками).
    pub fn push_frame(&mut self, f: FrameRecord) {
        if matches!(f.time, FrameTimestamp::Qpc(_)) {
            if self.frames.len() >= MAX_FRAMES {
                self.frames.pop_front();
            }
            self.frames.push_back(f);
        }
    }

    /// Додає знімок метрик.
    pub fn push_snapshot(&mut self, s: Snapshot) {
        self.snapshots.push_back(s);
    }

    /// Позначає стан гри на момент `now`: `focused` — у фокусі й малює кадри.
    /// Зарахування починається після прогріву [`RESUME_WARMUP`].
    pub fn mark(&mut self, now: i64, focused: bool) {
        if !focused {
            self.focus_since = None;
            if let Some(a) = self.open.take() {
                if now > a {
                    self.segments.push_back((a, now));
                }
            }
            return;
        }
        let since = *self.focus_since.get_or_insert(now);
        if self.open.is_none() && now - since >= self.warmup_ticks {
            self.open = Some(now);
        }
    }

    /// Чи йде зараз зарахована гра.
    pub fn in_game(&self) -> bool {
        self.open.is_some()
    }

    /// Відкидає все, старіше за `now − keep`.
    pub fn trim(&mut self, now: i64) {
        let horizon = now - self.keep_ticks;
        while self.frames.front().is_some_and(|f| qpc_of(f) < horizon) {
            self.frames.pop_front();
        }
        while self.snapshots.front().is_some_and(|s| s.qpc < horizon) {
            self.snapshots.pop_front();
        }
        while self.segments.front().is_some_and(|(_, b)| *b < horizon) {
            self.segments.pop_front();
        }
    }

    /// Зараховані відрізки в межах `[from, to]` (відкритий — до `to`).
    pub fn segments_in(&self, from: i64, to: i64) -> Vec<(i64, i64)> {
        self.segments
            .iter()
            .copied()
            .chain(self.open.map(|a| (a, to)))
            .filter_map(|(a, b)| {
                let (a, b) = (a.max(from), b.min(to));
                (b > a).then_some((a, b))
            })
            .collect()
    }

    /// Скільки зарахованої гри у вікні `[to − window, to]`, с.
    pub fn counted_s(&self, to: i64, window: Duration) -> f64 {
        let from = to - dur_ticks(window, self.freq);
        let ticks: i64 = self.segments_in(from, to).iter().map(|(a, b)| b - a).sum();
        ticks_to_ms(ticks, self.freq) / 1000.0
    }

    /// Вирізає вікно `[at − window, at]` як запис. `dpc` — трасування за те саме вікно.
    pub fn slice(
        &self,
        target: &Target,
        at: i64,
        window: Duration,
        start_unix_ms: Option<i64>,
        dpc: Option<optimax_core::etw::DpcTrace>,
        messages: Vec<String>,
    ) -> Result<Recording, CaptureFailure> {
        let from = at - dur_ticks(window, self.freq);
        let segments = self.segments_in(from, at);
        let counted: i64 = segments.iter().map(|(a, b)| b - a).sum();
        if counted < dur_ticks(MIN_GAME, self.freq) {
            return Err(CaptureFailure::NotEnoughGame);
        }
        let frames: Vec<FrameRecord> = self
            .frames
            .iter()
            .filter(|f| (from..=at).contains(&qpc_of(f)))
            .cloned()
            .collect();
        if frames.is_empty() {
            return Err(CaptureFailure::NoFrames);
        }
        // Знімки трохи ширше за вікно: правила дивляться ±200 мс навколо піків на краях.
        let margin = dur_ticks(Duration::from_secs(1), self.freq);
        let snapshots = self
            .snapshots
            .iter()
            .filter(|s| (from - margin..=at + margin).contains(&s.qpc))
            .cloned()
            .collect();
        Ok(Recording {
            target: target.clone(),
            qpc_frequency: self.freq,
            start_qpc: from,
            frames,
            snapshots,
            skipped_lines: 0,
            presentmon_messages: messages,
            cancelled: false,
            segments,
            end_reason: EndReason::Completed,
            start_unix_ms,
            dpc_trace: dpc,
        })
    }
}

fn qpc_of(f: &FrameRecord) -> i64 {
    match f.time {
        FrameTimestamp::Qpc(t) => t,
        FrameTimestamp::Ms(_) => i64::MIN,
    }
}

fn dur_ticks(d: Duration, freq: i64) -> i64 {
    i64::try_from(d.as_micros())
        .unwrap_or(i64::MAX / 2)
        .saturating_mul(freq)
        / 1_000_000
}

/// Запущене стеження. Зупиняється [`Watcher::stop`] або при знищенні.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    capture: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watcher")
    }
}

impl Watcher {
    /// Починає стеження: реєструє `Ctrl+Shift+F11` (помилка, якщо клавіша зайнята), запускає
    /// семплер, трасування DPC і PresentMon. Події — у повернутому каналі.
    pub fn start(
        presentmon: &BundledPresentMon,
        options: WatchOptions,
        system: Box<dyn SystemSource>,
        gpu: Option<Box<dyn GpuApi>>,
    ) -> Result<(Self, Receiver<WatchEvent>), FrametimeError> {
        options.target.validate()?;
        let hotkey =
            optimax_core::input::GlobalHotkey::ctrl_shift(optimax_core::input::FunctionKey::F11)
                .map_err(|e| FrametimeError::Hotkey(format!("Ctrl+Shift+F11: {e}")))?;
        let freq = qpc_frequency();
        let keep_secs = u32::try_from(KEEP.as_secs()).unwrap_or(40);
        let dpc = match optimax_core::etw::DpcTracer::start_ring(freq, keep_secs) {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!("трасування DPC недоступне: {e}");
                None
            }
        };
        let (sampler, snapshots) = Sampler::spawn(options.sampler, system, gpu)?;
        let pm = PmProcess::spawn(presentmon, &options.target)?;

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let capture = Arc::new(AtomicBool::new(false));
        let ctx = Loop {
            presentmon: presentmon.clone(),
            options,
            freq,
            hotkey,
            dpc,
            sampler: Some(sampler),
            snapshots,
            pm: Some(pm),
            stop: stop.clone(),
            capture: capture.clone(),
            tx,
        };
        let handle = std::thread::Builder::new()
            .name("optimax-lag-watch".into())
            .spawn(move || ctx.run())?;
        Ok((
            Self {
                stop,
                capture,
                handle: Some(handle),
            },
            rx,
        ))
    }

    /// Те саме, що натискання гарячої клавіші: зберегти останні секунди.
    pub fn capture_now(&self) {
        self.capture.store(true, Ordering::SeqCst);
    }

    /// Чи працює ще потік стеження.
    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Зупиняє стеження й чекає, поки все звільниться.
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

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Процес PresentMon із потоками читання.
struct PmProcess {
    child: Child,
    started: Instant,
    frames: Receiver<FrameRecord>,
    last_frame: Arc<AtomicI64>,
    messages: Arc<Mutex<Vec<String>>>,
}

impl PmProcess {
    fn spawn(presentmon: &BundledPresentMon, target: &Target) -> Result<Self, FrametimeError> {
        // Гри може ще не бути, а стеження довге: PresentMon сам не завершується з грою,
        // а після MAX_CAPTURE запускається знову.
        let mut child = presentmon.spawn_capture(target, MAX_CAPTURE, false)?;
        let (ftx, frames) = mpsc::channel();
        let last_frame = Arc::new(AtomicI64::new(0));
        let messages = Arc::new(Mutex::new(Vec::new()));
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        {
            let last_frame = last_frame.clone();
            std::thread::Builder::new()
                .name("optimax-lag-csv".into())
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
                            last_frame.store(qpc_now(), Ordering::Relaxed);
                            if ftx.send(f).is_err() {
                                return;
                            }
                        }
                    }
                })?;
        }
        {
            let messages = messages.clone();
            std::thread::Builder::new()
                .name("optimax-lag-err".into())
                .spawn(move || {
                    let Some(err) = stderr else { return };
                    for l in BufReader::new(err).lines().map_while(Result::ok) {
                        let l = l.trim();
                        if l.is_empty() {
                            continue;
                        }
                        let mut m = messages
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if m.len() < MAX_MESSAGES {
                            m.push(l.to_string());
                        }
                    }
                })?;
        }
        Ok(Self {
            child,
            started: Instant::now(),
            frames,
            last_frame,
            messages,
        })
    }

    fn messages(&self) -> Vec<String> {
        self.messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn shutdown(mut self, presentmon: &BundledPresentMon) {
        if matches!(self.child.try_wait(), Ok(None)) {
            if let Err(e) = presentmon.stop_session() {
                tracing::warn!("не вдалося зупинити ETW-сесію PresentMon: {e}");
            }
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
            }
        }
        let _ = self.child.wait();
    }
}

/// Стан основного циклу (живе в потоці стеження).
struct Loop {
    presentmon: BundledPresentMon,
    options: WatchOptions,
    freq: i64,
    hotkey: optimax_core::input::GlobalHotkey,
    dpc: Option<optimax_core::etw::DpcTracer>,
    sampler: Option<Sampler>,
    snapshots: Receiver<Snapshot>,
    pm: Option<PmProcess>,
    stop: Arc<AtomicBool>,
    capture: Arc<AtomicBool>,
    tx: Sender<WatchEvent>,
}

/// Unix-час (мс), що відповідає мітці QPC `qpc` (за поточним годинником).
fn unix_ms_at(qpc: i64, freq: i64) -> Option<i64> {
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())?;
    let back = ticks_to_ms(qpc_now() - qpc, freq);
    Some(now_unix - back.round() as i64)
}

impl Loop {
    fn run(mut self) {
        let end = self.watch();
        if let Some(pm) = self.pm.take() {
            pm.shutdown(&self.presentmon);
        }
        if let Some(s) = self.sampler.take() {
            s.stop();
        }
        if let Some(t) = self.dpc.take() {
            drop(t.stop());
        }
        let _ = self.tx.send(WatchEvent::Ended(end));
    }

    /// Повідомлення поверх гри: «зберігаю» або (`failed`) «замало гри».
    fn notice(&self, toaster: Option<&Toaster>, failed: bool) {
        if let (Some(t), Some(n)) = (toaster, &self.options.notices) {
            if failed {
                t.show(&n.too_short, Tone::Warn, NOTICE);
            } else {
                t.show(&n.saving, Tone::Info, NOTICE);
            }
        }
    }

    fn watch(&mut self) -> WatchEnd {
        let window = self.options.window();
        let presenting_ticks = dur_ticks(PRESENTING_WINDOW, self.freq);
        let mut ring = Ring::new(self.freq, KEEP);
        let toaster = self
            .options
            .notices
            .as_ref()
            .and_then(|_| Toaster::new().ok());
        let mut pids = game_pids(&self.options.target);
        let mut seen_running = pids.as_ref().is_some_and(|p| !p.is_empty());
        let mut last_pid_check = Instant::now();
        let mut last_status: Option<Instant> = None;
        let mut pending: Option<(i64, Instant, Option<i64>)> = None;
        let mut quick_exits = 0u32;

        loop {
            if self.stop.load(Ordering::SeqCst) {
                return WatchEnd::Stopped;
            }
            if let Ok(s) = self.snapshots.recv_timeout(TICK) {
                ring.push_snapshot(s);
            }
            let Some(pm) = self.pm.as_mut() else {
                return WatchEnd::Stopped;
            };
            for f in pm.frames.try_iter() {
                ring.push_frame(f);
            }
            // PresentMon завершився (MAX_CAPTURE або збій) — перезапускаємо.
            if !matches!(pm.child.try_wait(), Ok(None)) {
                let quick = pm.started.elapsed() < QUICK_EXIT;
                let messages = pm.messages();
                if let Some(old) = self.pm.take() {
                    old.shutdown(&self.presentmon);
                }
                quick_exits = if quick { quick_exits + 1 } else { 0 };
                if quick_exits >= MAX_QUICK_EXITS {
                    return WatchEnd::PresentMonFailed(messages.join(" | "));
                }
                match PmProcess::spawn(&self.presentmon, &self.options.target) {
                    Ok(p) => self.pm = Some(p),
                    Err(e) => return WatchEnd::PresentMonFailed(e.to_string()),
                }
                continue;
            }
            if last_pid_check.elapsed() >= PROCESS_CHECK {
                pids = game_pids(&self.options.target);
                last_pid_check = Instant::now();
            }
            let running = pids.as_ref().is_none_or(|p| !p.is_empty());
            let now = qpc_now();
            let focused = match (&pids, optimax_core::input::foreground_process_id()) {
                (Some(p), Some(fg)) => p.contains(&fg),
                _ => false,
            };
            let last = pm.last_frame.load(Ordering::Relaxed);
            let presenting = last > 0 && now - last <= presenting_ticks;
            ring.mark(now, running && focused && presenting);

            if (self.hotkey.pressed() || self.capture.swap(false, Ordering::SeqCst))
                && pending.is_none()
            {
                if self.options.sound {
                    optimax_core::input::beep_start();
                }
                self.notice(toaster.as_ref(), false);
                pending = Some((now, Instant::now(), unix_ms_at(now, self.freq)));
                tracing::info!("«Щойно лагнуло»: натиснуто, зберігаю");
                if self.tx.send(WatchEvent::Pressed).is_err() {
                    return WatchEnd::Stopped;
                }
            }
            if let Some((at, pressed, at_unix_ms)) = pending {
                if pressed.elapsed() >= DRAIN {
                    pending = None;
                    let from = at - dur_ticks(window, self.freq);
                    let dpc = self.dpc.as_ref().and_then(|t| t.window(from, at));
                    let start_unix_ms =
                        at_unix_ms.map(|u| u - ticks_to_ms(at - from, self.freq).round() as i64);
                    let messages = self
                        .pm
                        .as_ref()
                        .map(PmProcess::messages)
                        .unwrap_or_default();
                    let event = match ring.slice(
                        &self.options.target,
                        at,
                        window,
                        start_unix_ms,
                        dpc,
                        messages,
                    ) {
                        Ok(rec) => WatchEvent::Captured {
                            recording: Box::new(rec),
                            at_unix_ms,
                        },
                        Err(reason) => {
                            self.notice(toaster.as_ref(), true);
                            WatchEvent::CaptureFailed { reason, at_unix_ms }
                        }
                    };
                    if self.tx.send(event).is_err() {
                        return WatchEnd::Stopped;
                    }
                }
            }
            ring.trim(now);

            if running {
                seen_running = true;
            } else if seen_running && pending.is_none() {
                return WatchEnd::GameExited;
            }
            if last_status.is_none_or(|t| t.elapsed() >= STATUS_EVERY) {
                last_status = Some(Instant::now());
                let status = WatchStatus {
                    game_running: running,
                    in_game: ring.in_game(),
                    buffered_s: ring.counted_s(now, window),
                    dpc: self.dpc.is_some(),
                };
                if self.tx.send(WatchEvent::Status(status)).is_err() {
                    return WatchEnd::Stopped;
                }
            }
        }
    }
}

/// PID процесів гри (`None` — визначити не вдалося, тоді вважаємо гру запущеною).
fn game_pids(target: &Target) -> Option<Vec<u32>> {
    match target {
        #[cfg(windows)]
        Target::ProcessName(name) => {
            use optimax_core::process::{ProcessApi, WinProcesses};
            WinProcesses
                .find_by_name(name)
                .ok()
                .map(|v| v.into_iter().map(|p| p.pid).collect())
        }
        #[cfg(not(windows))]
        Target::ProcessName(_) => None,
        Target::ProcessId(pid) => Some(vec![*pid]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1000 тиків = 1 с.
    const F: i64 = 1000;

    fn frame(qpc: i64) -> FrameRecord {
        FrameRecord {
            application: "g.exe".into(),
            process_id: 1,
            swap_chain: "0x1".into(),
            present_mode: String::new(),
            time: FrameTimestamp::Qpc(qpc),
            ms_between_presents: 10.0,
            ms_between_display_change: None,
            ms_cpu_busy: None,
            ms_gpu_busy: None,
            sync_interval: None,
            allows_tearing: None,
        }
    }

    fn target() -> Target {
        Target::ProcessName("g.exe".into())
    }

    /// Гра у фокусі з `from` до `to` с (кадр кожні 10 мс), позначки кожні 100 мс.
    fn play(ring: &mut Ring, from: i64, to: i64) {
        for t in (from * F..to * F).step_by(10) {
            ring.push_frame(frame(t));
            if t % 100 == 0 {
                ring.mark(t, true);
            }
        }
    }

    #[test]
    fn counts_only_focused_game_after_warmup() {
        let mut ring = Ring::new(F, KEEP);
        play(&mut ring, 0, 20);
        ring.mark(20 * F, false); // Alt+Tab
        ring.mark(25 * F, true); // повернувся — знову прогрів 2 с
        play(&mut ring, 25, 40);
        // 0–20 с: зараховано з 2-ї секунди (прогрів); 25–40: з 27-ї.
        let segs = ring.segments_in(0, 40 * F);
        assert_eq!(segs, vec![(2 * F, 20 * F), (27 * F, 40 * F)]);
        assert!(ring.in_game());
        // У вікні 30 с до 40-ї секунди: 10–20 (10 с) + 27–40 (13 с).
        assert!((ring.counted_s(40 * F, Duration::from_secs(30)) - 23.0).abs() < 1e-9);
    }

    #[test]
    fn slice_cuts_window_and_keeps_segments_inside() {
        let mut ring = Ring::new(F, KEEP);
        play(&mut ring, 0, 50);
        ring.trim(50 * F);
        // Буфер тримає лише останні 40 с.
        assert!(ring.frames.front().is_some_and(|f| qpc_of(f) >= 10 * F));
        let rec = ring
            .slice(
                &target(),
                50 * F,
                Duration::from_secs(30),
                Some(1_000),
                None,
                vec![],
            )
            .unwrap();
        assert_eq!(rec.start_qpc, 20 * F);
        assert_eq!(rec.segments, vec![(20 * F, 50 * F)]);
        assert!((rec.counted_seconds() - 30.0).abs() < 1e-9);
        assert!(rec
            .frames
            .iter()
            .all(|f| (20 * F..=50 * F).contains(&qpc_of(f))));
        assert_eq!(rec.frames.len(), 3000);
        assert_eq!(rec.start_unix_ms, Some(1_000));
    }

    #[test]
    fn too_little_game_or_no_frames_is_reported() {
        let mut ring = Ring::new(F, KEEP);
        // Лише 4 с гри (з прогрівом — 2 с зараховано).
        play(&mut ring, 0, 4);
        ring.mark(4 * F, false);
        assert_eq!(
            ring.slice(
                &target(),
                30 * F,
                Duration::from_secs(30),
                None,
                None,
                vec![]
            )
            .unwrap_err(),
            CaptureFailure::NotEnoughGame
        );
        // Гра «у фокусі», але кадрів у вікні немає.
        let mut empty = Ring::new(F, KEEP);
        for t in (0..20 * F).step_by(100) {
            empty.mark(t, true);
        }
        assert_eq!(
            empty
                .slice(
                    &target(),
                    20 * F,
                    Duration::from_secs(30),
                    None,
                    None,
                    vec![]
                )
                .unwrap_err(),
            CaptureFailure::NoFrames
        );
    }

    #[test]
    fn frames_without_qpc_are_ignored_and_buffer_is_bounded() {
        let mut ring = Ring::new(F, KEEP);
        let mut f = frame(0);
        f.time = FrameTimestamp::Ms(5.0);
        ring.push_frame(f);
        assert!(ring.frames.is_empty());
        for t in 0..(MAX_FRAMES as i64 + 10) {
            ring.push_frame(frame(t));
        }
        assert_eq!(ring.frames.len(), MAX_FRAMES);
    }

    #[test]
    fn window_is_clamped() {
        let mut o = WatchOptions::new(target());
        o.window = Duration::from_secs(1);
        assert_eq!(o.window(), MIN_GAME);
        o.window = Duration::from_secs(600);
        assert_eq!(o.window(), Duration::from_secs(35));
    }
}
