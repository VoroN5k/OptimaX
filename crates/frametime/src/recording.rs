//! Одночасний запис кадрів (PresentMon) і системних метрик на спільній шкалі QPC.
//!
//! PresentMon і метрики пишуть увесь час, а [`Gate`] позначає відрізки чистої гри:
//! у режимі [`StartMode::Auto`] — коли гра на передньому плані й малює кадри (після прогріву),
//! у [`StartMode::Hotkey`] — між натисканнями Ctrl+Shift+F9. В аналіз іде лише позначене.

use crate::csv::CsvFrameParser;
use crate::frame::{FrameRecord, FrameTimestamp};
use crate::gate::{EndReason, Gate, Observation, Phase, StartMode, PRESENTING_WINDOW};
use crate::presentmon::{BundledPresentMon, Target, MAX_CAPTURE};
use crate::FrametimeError;
use optimax_core::clock::{qpc_frequency, qpc_now, ticks_to_ms};
use optimax_metrics::{GpuApi, Sampler, SamplerConfig, Snapshot, SystemSource};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Скільки рядків stderr PresentMon зберігати для звіту.
const MAX_WARNINGS: usize = 20;
/// Такт шлюзу.
const TICK: Duration = Duration::from_millis(100);
/// Як часто перевіряти, чи процес гри ще живий (знімок списку процесів дорожчий за такт).
const PROCESS_CHECK: Duration = Duration::from_millis(500);
/// Як часто повідомляти про хід запису.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// Найдовша тривалість зарахованої гри.
pub const MAX_DURATION: Duration = Duration::from_secs(600);
/// Скільки ще слухати PresentMon після завершення: кадри приходять із затримкою ~1–2 с,
/// і без цього останні секунди зарахованого відрізка загубилися б.
const DRAIN: Duration = Duration::from_millis(2500);
/// Скільки за замовчуванням чекати, поки гра з'явиться / натиснуть клавішу.
pub const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(600);

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
    /// Скільки чистої гри зарахувати (5 с – 10 хв); у режимі клавіші — максимум.
    pub duration: Duration,
    /// Як починати.
    pub start: StartMode,
    /// Скільки чекати старту (гра в фокусі / натискання клавіші).
    pub max_wait: Duration,
    /// Звукові сигнали на старті й зупинці.
    pub sound: bool,
    /// Налаштування семплера метрик. Для кореляції з піками краще 200 мс.
    pub sampler: SamplerConfig,
}

impl RecordOptions {
    /// Запис одразу (як у консольних інструментах): 200 мс між знімками метрик.
    pub fn new(target: Target, duration: Duration) -> Self {
        Self {
            target,
            duration: duration.clamp(Duration::from_secs(5), MAX_DURATION),
            start: StartMode::Immediate,
            max_wait: DEFAULT_MAX_WAIT,
            sound: false,
            sampler: SamplerConfig {
                interval: Duration::from_millis(200),
                process_interval: Duration::from_secs(1),
                top_processes: 8,
            },
        }
    }

    /// Те саме з вибраним способом старту.
    #[must_use]
    pub fn with_start(mut self, start: StartMode, sound: bool) -> Self {
        self.start = start;
        self.sound = sound;
        self
    }
}

/// Хід запису для інтерфейсу.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecordProgress {
    /// Фаза.
    pub phase: Phase,
    /// Зараховано чистої гри, с.
    pub counted_s: f64,
    /// Потрібно, с.
    pub target_s: f64,
    /// Минуло від старту, с.
    pub elapsed_s: f64,
}

fn default_end() -> EndReason {
    EndReason::Completed
}

/// Результат запису.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// Ціль.
    pub target: Target,
    /// Частота QPC.
    pub qpc_frequency: i64,
    /// QPC на момент старту запису — нуль шкали часу.
    pub start_qpc: i64,
    /// Усі кадри (усіх swap chain цілі), зокрема поза зарахованими відрізками.
    pub frames: Vec<FrameRecord>,
    /// Знімки метрик.
    pub snapshots: Vec<Snapshot>,
    /// Рядки CSV, які не вдалося розібрати.
    pub skipped_lines: usize,
    /// Повідомлення PresentMon зі stderr (попередження, помилки доступу).
    pub presentmon_messages: Vec<String>,
    /// Запис зупинено користувачем раніше часу.
    pub cancelled: bool,
    /// Зараховані відрізки (QPC). Порожньо — увесь запис (старі файли / режим «одразу»).
    #[serde(default)]
    pub segments: Vec<(i64, i64)>,
    /// Чому запис завершився.
    #[serde(default = "default_end")]
    pub end_reason: EndReason,
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

    /// Чи потрапляє мітка QPC у зараховані відрізки (без відрізків — так).
    pub fn in_segments(&self, qpc: i64) -> bool {
        self.segments.is_empty() || self.segments.iter().any(|(a, b)| qpc >= *a && qpc < *b)
    }

    /// Чи потрапляє кадр у зараховані відрізки.
    pub fn frame_counted(&self, frame: &FrameRecord) -> bool {
        match frame.time {
            FrameTimestamp::Qpc(t) => self.in_segments(t),
            // Без QPC відрізки не зіставити — кадр не відкидаємо.
            FrameTimestamp::Ms(_) => true,
        }
    }

    /// Зараховано чистої гри, с.
    pub fn counted_seconds(&self) -> f64 {
        let ticks: i64 = self.segments.iter().map(|(a, b)| b - a).sum();
        ticks_to_ms(ticks, self.qpc_frequency) / 1000.0
    }
}

/// Записує кадри й метрики, не повідомляючи про хід. Див. [`record_with`].
pub fn record(
    presentmon: &BundledPresentMon,
    options: &RecordOptions,
    system: Box<dyn SystemSource>,
    gpu: Option<Box<dyn GpuApi>>,
    cancel: &CancelToken,
) -> Result<Recording, FrametimeError> {
    record_with(presentmon, options, system, gpu, cancel, &mut |_| {})
}

/// Чи запущено процес гри (для цілі за PID — вважаємо, що так, поки працює PresentMon).
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

/// Записує кадри й метрики. Блокує потік на час запису — викликати не з UI-потоку.
pub fn record_with(
    presentmon: &BundledPresentMon,
    options: &RecordOptions,
    system: Box<dyn SystemSource>,
    gpu: Option<Box<dyn GpuApi>>,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(&RecordProgress),
) -> Result<Recording, FrametimeError> {
    options.target.validate()?;
    let duration = options.duration.clamp(Duration::from_secs(5), MAX_DURATION);
    let immediate = options.start == StartMode::Immediate;
    // Гарячу клавішу реєструємо до старту: якщо зайнята — кажемо одразу.
    let hotkey = if options.start == StartMode::Hotkey {
        Some(
            optimax_core::input::GlobalHotkey::ctrl_shift_f9()
                .map_err(|e| FrametimeError::Hotkey(e.to_string()))?,
        )
    } else {
        None
    };
    // PresentMon працює весь час очікування й запису; зупиняємо його самі.
    let pm_duration = if immediate {
        duration
    } else {
        (options.max_wait + duration * 2 + Duration::from_secs(60)).min(MAX_CAPTURE)
    };

    let freq = qpc_frequency();
    let start_qpc = qpc_now();
    let (sampler, snapshots_rx) = Sampler::spawn(options.sampler, system, gpu)?;
    let mut child = presentmon.spawn_capture(&options.target, pm_duration, immediate)?;

    let last_frame = Arc::new(AtomicI64::new(0));
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let frames_thread = {
        let last_frame = last_frame.clone();
        std::thread::Builder::new()
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
                        Ok(Some(f)) => {
                            // Час надходження (а не показу) — див. PRESENTING_WINDOW.
                            last_frame.store(qpc_now(), Ordering::Relaxed);
                            frames.push(f);
                        }
                        Ok(None) => {}
                        Err(_) => skipped += 1,
                    }
                }
                (frames, skipped)
            })?
    };
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

    let mut gate = Gate::new(options.start, duration, options.max_wait, freq, start_qpc);
    let presenting_ticks =
        i64::try_from(PRESENTING_WINDOW.as_micros()).unwrap_or(0) * freq / 1_000_000;
    let started = Instant::now();
    let mut snapshots = Vec::new();
    let mut pids: Option<Vec<u32>> = game_pids(&options.target);
    let mut last_pid_check = Instant::now();
    let mut last_progress: Option<Instant> = None;
    let mut last_phase = gate.phase();
    let mut beeped_start = false;
    let mut pm_exited = false;

    while gate.phase() != Phase::Done {
        if let Ok(s) = snapshots_rx.recv_timeout(TICK) {
            snapshots.push(s);
        }
        if child.try_wait()?.is_some() {
            pm_exited = true;
        }
        if last_pid_check.elapsed() >= PROCESS_CHECK {
            pids = game_pids(&options.target);
            last_pid_check = Instant::now();
        }
        let now = qpc_now();
        let running = match &pids {
            Some(p) => !p.is_empty(),
            None => true,
        } && !pm_exited;
        let focused = match (&pids, optimax_core::input::foreground_process_id()) {
            (Some(p), Some(fg)) => p.contains(&fg),
            _ => false,
        };
        let last = last_frame.load(Ordering::Relaxed);
        let obs = Observation {
            game_running: running,
            game_focused: focused,
            game_presenting: last > 0 && now - last <= presenting_ticks,
            hotkey: hotkey.as_ref().is_some_and(|h| h.pressed()),
            cancel: cancel.is_cancelled(),
        };
        let phase = gate.tick(now, &obs);
        if options.sound && phase == Phase::Recording && !beeped_start && !immediate {
            optimax_core::input::beep_start();
            beeped_start = true;
        }
        if phase != last_phase || last_progress.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
            on_progress(&RecordProgress {
                phase,
                counted_s: gate.counted_ms(now) / 1000.0,
                target_s: duration.as_secs_f64(),
                elapsed_s: started.elapsed().as_secs_f64(),
            });
            last_progress = Some(Instant::now());
            last_phase = phase;
        }
        // Режим «одразу»: PresentMon сам завершується за --timed / при виході гри.
        if immediate && pm_exited {
            break;
        }
    }

    let end_reason = gate.end_reason().unwrap_or(EndReason::Completed);
    if options.sound && !immediate && matches!(end_reason, EndReason::Completed | EndReason::Hotkey)
    {
        optimax_core::input::beep_stop();
    }
    // Дописуємо хвіст: кадри останніх секунд відрізка ще в дорозі.
    if !immediate && gate.has_recorded() && end_reason != EndReason::GameExited {
        let drain_until = Instant::now() + DRAIN;
        while Instant::now() < drain_until && child.try_wait()?.is_none() {
            if let Ok(s) = snapshots_rx.recv_timeout(TICK) {
                snapshots.push(s);
            }
        }
    }
    if child.try_wait()?.is_none() {
        if let Err(e) = presentmon.stop_session() {
            tracing::warn!("не вдалося зупинити ETW-сесію: {e}");
        }
        if child.try_wait()?.is_none() {
            let _ = child.kill();
        }
    }
    let _ = child.wait();
    drop(hotkey);
    sampler.stop();
    snapshots.extend(snapshots_rx.try_iter());

    let (frames, skipped_lines) = frames_thread.join().unwrap_or_default();
    let presentmon_messages = messages_thread.join().unwrap_or_default();
    if !gate.has_recorded() {
        return Err(FrametimeError::NotStarted(end_reason));
    }
    let segments = if immediate {
        Vec::new()
    } else {
        gate.segments().to_vec()
    };
    let rec = Recording {
        target: options.target.clone(),
        qpc_frequency: freq,
        start_qpc,
        frames,
        snapshots,
        skipped_lines,
        presentmon_messages,
        cancelled: end_reason == EndReason::Cancelled,
        segments,
        end_reason,
    };
    if !rec.frames.iter().any(|f| rec.frame_counted(f)) {
        return Err(FrametimeError::NoFrames(
            rec.presentmon_messages.join(" | "),
        ));
    }
    Ok(rec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(qpc: i64) -> FrameRecord {
        FrameRecord {
            application: "g.exe".into(),
            process_id: 1,
            swap_chain: "0x1".into(),
            present_mode: String::new(),
            time: FrameTimestamp::Qpc(qpc),
            ms_between_presents: 8.0,
            ms_between_display_change: None,
            ms_cpu_busy: None,
            ms_gpu_busy: None,
        }
    }

    #[test]
    fn segments_filter_frames_and_count_time() {
        let rec = Recording {
            target: Target::ProcessName("g.exe".into()),
            qpc_frequency: 1000,
            start_qpc: 0,
            frames: vec![frame(500), frame(1500), frame(2500), frame(4500)],
            snapshots: vec![],
            skipped_lines: 0,
            presentmon_messages: vec![],
            cancelled: false,
            segments: vec![(1000, 3000), (4000, 5000)],
            end_reason: EndReason::Completed,
        };
        let counted: Vec<bool> = rec.frames.iter().map(|f| rec.frame_counted(f)).collect();
        assert_eq!(counted, vec![false, true, true, true]);
        assert!((rec.counted_seconds() - 3.0).abs() < 1e-9);
        // Без відрізків — увесь запис.
        let all = Recording {
            segments: vec![],
            ..rec
        };
        assert!(all.frames.iter().all(|f| all.frame_counted(f)));
    }

    #[test]
    fn old_recordings_without_segments_still_load() {
        let json = r#"{"target":{"ProcessName":"g.exe"},"qpc_frequency":1000,"start_qpc":0,
            "frames":[],"snapshots":[],"skipped_lines":0,"presentmon_messages":[],"cancelled":false}"#;
        let rec: Recording = serde_json::from_str(json).unwrap();
        assert!(rec.segments.is_empty());
        assert_eq!(rec.end_reason, EndReason::Completed);
    }
}
