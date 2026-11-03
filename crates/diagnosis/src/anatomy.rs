//! Анатомія кадру: з чого складається типовий кадр і де саме виникла затримка в ривках.
//!
//! Джерело — стовпці PresentMon 2.x (див. `optimax_frametime::FramePipeline`):
//! інтервал між Present кадру `i` = `MsCPUWait(i−1)` (CPU чекав після попереднього Present) +
//! `MsCPUBusy(i)` (CPU готував цей кадр) — перевірено на справжньому виводі PresentMon 2.6.0.
//! Тож для кожного ривка видно, яка з двох частин розрослась; робота GPU над кадром
//! (`MsGPUBusy`, `MsGPUWait`, `MsGPULatency`) показує, чи чекав CPU саме на відеокарту.
//!
//! Це опис вимірів, а не вердикт: висновки про причини — у правилах ([`crate::rules`]).

use crate::rules::Analysis;
use crate::timeline::FramePoint;
use serde::{Deserialize, Serialize};

/// Скільки найгірших ривків розбирати.
pub const MAX_SPIKES: usize = 20;
/// GPU вважається «важким» у ривку, якщо працював щонайменше таку частку тривалості ривка.
const GPU_HEAVY_SHARE: f64 = 0.5;

/// Частини кадру, мс (`None` — PresentMon не дав значення).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FrameParts {
    /// Тривалість кадру (між Present), мс.
    pub frame_ms: f64,
    /// CPU готував кадр.
    pub cpu_busy: Option<f64>,
    /// CPU чекав після попереднього Present, перш ніж почати цей кадр.
    pub cpu_wait: Option<f64>,
    /// Від початку кадру на CPU до початку роботи GPU над ним.
    pub gpu_latency: Option<f64>,
    /// GPU активно працював над кадром.
    pub gpu_busy: Option<f64>,
    /// GPU простоював посеред роботи над кадром.
    pub gpu_wait: Option<f64>,
    /// Від Present до показу на екрані.
    pub until_displayed: Option<f64>,
}

/// Що розтягнуло ривок.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Stall {
    /// CPU довше готував кадр (робота гри: логіка, потокове завантаження, шейдери…).
    CpuBusy,
    /// CPU чекав після попереднього кадру, а відеокарта в цей час довго працювала.
    WaitGpu,
    /// CPU чекав після попереднього кадру без важкої роботи GPU (синхронізація, драйвер,
    /// ліміт, показ).
    Wait,
}

/// Що робив потік протягом кадру ривка, мс (з точних проміжків трасування).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThreadSlice {
    /// Працював.
    pub running: f64,
    /// Готовий, чекав на вільне ядро.
    pub ready: f64,
    /// Заблокований на синхронізації.
    pub sync: f64,
    /// Чекав на сторінку з диска.
    pub page_in: f64,
    /// Спав.
    pub sleep: f64,
    /// Інше очікування ядра.
    pub other: f64,
}

/// Стан потоку, що забрав найбільше часу кадру.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    /// Працював (обчислення).
    Running,
    /// Чекав на вільне ядро (його витіснили).
    Ready,
    /// Чекав на інший потік, подію чи блокування.
    Sync,
    /// Чекав на сторінку з диска (page fault).
    PageIn,
    /// Спав.
    Sleep,
    /// Інше очікування ядра.
    Other,
}

/// Частка кадру, яку мають покривати проміжки, щоб робити висновок.
const MIN_COVERAGE: f64 = 0.5;

impl ThreadSlice {
    fn total(&self) -> f64 {
        self.running + self.ready + self.sync + self.page_in + self.sleep + self.other
    }

    /// Стан, що забрав найбільше часу.
    pub fn dominant(&self) -> ThreadState {
        [
            (ThreadState::Running, self.running),
            (ThreadState::Ready, self.ready),
            (ThreadState::Sync, self.sync),
            (ThreadState::PageIn, self.page_in),
            (ThreadState::Sleep, self.sleep),
            (ThreadState::Other, self.other),
        ]
        .into_iter()
        .fold((ThreadState::Running, -1.0), |best, x| {
            if x.1 > best.1 {
                x
            } else {
                best
            }
        })
        .0
    }
}

/// Скільки часу потоку в кожному стані у вікні `[from, to]` (мс). `None`, якщо проміжки
/// покривають менше половини вікна (потік не трасувався чи вікно поза детальними даними).
pub fn thread_slice(detail: &[(f64, f64, u8)], from: f64, to: f64) -> Option<ThreadSlice> {
    use optimax_trace::threads::slot;
    let mut s = ThreadSlice::default();
    let start = detail.partition_point(|(a, d, _)| a + d <= from);
    for &(a, d, st) in &detail[start..] {
        if a >= to {
            break;
        }
        let part = (a + d).min(to) - a.max(from);
        if part <= 0.0 {
            continue;
        }
        match usize::from(st) {
            slot::RUNNING => s.running += part,
            slot::READY => s.ready += part,
            slot::SYNC => s.sync += part,
            slot::PAGE_IN => s.page_in += part,
            slot::SLEEP => s.sleep += part,
            _ => s.other += part,
        }
    }
    (to > from && s.total() >= (to - from) * MIN_COVERAGE).then_some(s)
}

/// Розбір одного ривка.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SpikeAnatomy {
    /// Час кадру, мс від старту запису.
    pub t_ms: f64,
    /// Частини цього кадру (`cpu_wait` — очікування після попереднього кадру).
    pub parts: FrameParts,
    /// Робота GPU над попереднім кадром, мс (на неї міг чекати CPU).
    pub prev_gpu_busy: Option<f64>,
    /// Що розтягнуло ривок (`None` — PresentMon не дав частин).
    pub stall: Option<Stall>,
    /// Що робив головний потік гри протягом кадру (трасування потоків).
    #[serde(default)]
    pub main: Option<ThreadSlice>,
    /// Стан головного потоку, що забрав найбільше часу кадру.
    #[serde(default)]
    pub main_state: Option<ThreadState>,
    /// Що робив рендер-потік (Present) протягом кадру.
    #[serde(default)]
    pub render: Option<ThreadSlice>,
}

/// Анатомія кадрів запису.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FrameAnatomy {
    /// Типовий кадр — медіани частин за всі кадри.
    pub typical: FrameParts,
    /// Скільки кадрів розібрано.
    pub frames: u32,
    /// Частка кадрів, у яких GPU почав роботу пізніше, ніж через тривалість кадру від його
    /// початку (кадри стоять у черзі — це додає затримки вводу).
    pub queued_share: Option<f64>,
    /// Найгірші ривки, найдовші першими.
    pub spikes: Vec<SpikeAnatomy>,
    /// Скільки розібраних ривків за кожною причиною: CPU готував / чекав на GPU / чекав.
    pub stall_counts: [u32; 3],
    /// Скільки ривків за станом головного потоку: працював / чекав на ядро / синхронізація /
    /// диск / сон / інше (лише ривки з даними трасування).
    #[serde(default)]
    pub main_counts: [u32; 6],
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

fn parts(f: &FramePoint, prev: Option<&FramePoint>) -> FrameParts {
    FrameParts {
        frame_ms: f.frametime_ms,
        cpu_busy: f.cpu_busy_ms,
        cpu_wait: prev.and_then(|p| p.pipeline.cpu_wait),
        gpu_latency: f.pipeline.gpu_latency,
        gpu_busy: f.gpu_busy_ms,
        gpu_wait: f.pipeline.gpu_wait,
        until_displayed: f.pipeline.until_displayed,
    }
}

/// Що розтягнуло кадр: більша з двох частин інтервалу між Present.
pub fn stall_of(p: &FrameParts, prev_gpu_busy: Option<f64>) -> Option<Stall> {
    let (busy, wait) = (p.cpu_busy?, p.cpu_wait?);
    if busy >= wait {
        return Some(Stall::CpuBusy);
    }
    let gpu = prev_gpu_busy.unwrap_or(0.0).max(p.gpu_busy.unwrap_or(0.0));
    Some(if gpu >= p.frame_ms * GPU_HEAVY_SHARE {
        Stall::WaitGpu
    } else {
        Stall::Wait
    })
}

/// Анатомія кадрів. `None`, якщо PresentMon не дав розкладу кадру (стара версія, 1.x).
pub fn frame_anatomy(a: &Analysis<'_>) -> Option<FrameAnatomy> {
    let frames = &a.timeline.frames;
    if !frames.iter().any(|f| f.cpu_busy_ms.is_some()) {
        return None;
    }
    let all: Vec<FrameParts> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| parts(f, i.checked_sub(1).and_then(|j| frames.get(j))))
        .collect();
    let med = |get: fn(&FrameParts) -> Option<f64>| median(all.iter().filter_map(get).collect());
    let typical = FrameParts {
        frame_ms: median(all.iter().map(|p| p.frame_ms).collect()).unwrap_or(0.0),
        cpu_busy: med(|p| p.cpu_busy),
        cpu_wait: med(|p| p.cpu_wait),
        gpu_latency: med(|p| p.gpu_latency),
        gpu_busy: med(|p| p.gpu_busy),
        gpu_wait: med(|p| p.gpu_wait),
        until_displayed: med(|p| p.until_displayed),
    };
    let with_latency: Vec<&FrameParts> = all.iter().filter(|p| p.gpu_latency.is_some()).collect();
    let queued_share = (!with_latency.is_empty()).then(|| {
        with_latency
            .iter()
            .filter(|p| p.gpu_latency.unwrap_or(0.0) > p.frame_ms)
            .count() as f64
            / with_latency.len() as f64
    });

    let mut worst: Vec<(f64, f64)> = a.spikes.iter().map(|(t, ms, _)| (*t, *ms)).collect();
    worst.sort_by(|x, y| y.1.total_cmp(&x.1));
    let mut stall_counts = [0u32; 3];
    let mut main_counts = [0u32; 6];
    let threads = a.timeline.threads.as_ref();
    let spikes: Vec<SpikeAnatomy> = worst
        .into_iter()
        .take(MAX_SPIKES)
        .filter_map(|(t, _)| {
            // Кадр ривка — той, чий Present у момент `t`.
            let i = frames.partition_point(|f| f.t_ms < t - 1e-6);
            let f = frames.get(i)?;
            let prev = i.checked_sub(1).and_then(|j| frames.get(j));
            let p = parts(f, prev);
            let prev_gpu_busy = prev.and_then(|q| q.gpu_busy_ms);
            let stall = stall_of(&p, prev_gpu_busy);
            if let Some(s) = stall {
                stall_counts[s as usize] += 1;
            }
            // Кадр ривка: від Present попереднього кадру до Present цього.
            let (from, to) = (f.t_ms - f.frametime_ms, f.t_ms);
            let main = threads.and_then(|t| thread_slice(&t.main_detail, from, to));
            let render = threads.and_then(|t| thread_slice(&t.render_detail, from, to));
            if let Some(m) = main {
                main_counts[m.dominant() as usize] += 1;
            }
            Some(SpikeAnatomy {
                t_ms: f.t_ms,
                parts: p,
                prev_gpu_busy,
                stall,
                main,
                main_state: main.map(|m| m.dominant()),
                render,
            })
        })
        .collect();
    Some(FrameAnatomy {
        typical,
        frames: u32::try_from(frames.len()).unwrap_or(u32::MAX),
        queued_share,
        spikes,
        stall_counts,
        main_counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(frame: f64, busy: f64, wait: f64, gpu: f64) -> FrameParts {
        FrameParts {
            frame_ms: frame,
            cpu_busy: Some(busy),
            cpu_wait: Some(wait),
            gpu_busy: Some(gpu),
            ..Default::default()
        }
    }

    #[test]
    fn thread_slice_sums_overlap_and_requires_coverage() {
        use optimax_trace::threads::slot;
        let detail = vec![
            (0.0, 10.0, slot::RUNNING as u8),
            (10.0, 40.0, slot::PAGE_IN as u8),
            (50.0, 20.0, slot::RUNNING as u8),
        ];
        let s = thread_slice(&detail, 5.0, 60.0).expect("покрито");
        assert!((s.running - 15.0).abs() < 1e-9);
        assert!((s.page_in - 40.0).abs() < 1e-9);
        assert_eq!(s.dominant(), ThreadState::PageIn);
        // Поза даними — без висновку.
        assert!(thread_slice(&detail, 100.0, 160.0).is_none());
        assert!(thread_slice(&detail, 60.0, 200.0).is_none());
    }

    #[test]
    fn stall_is_the_bigger_part_and_gpu_decides_the_wait() {
        assert_eq!(
            stall_of(&p(80.0, 70.0, 10.0, 5.0), None),
            Some(Stall::CpuBusy)
        );
        assert_eq!(
            stall_of(&p(80.0, 10.0, 70.0, 5.0), Some(60.0)),
            Some(Stall::WaitGpu)
        );
        assert_eq!(
            stall_of(&p(80.0, 10.0, 70.0, 50.0), Some(5.0)),
            Some(Stall::WaitGpu)
        );
        assert_eq!(
            stall_of(&p(80.0, 10.0, 70.0, 5.0), Some(5.0)),
            Some(Stall::Wait)
        );
        let mut no_cpu = p(80.0, 0.0, 0.0, 0.0);
        no_cpu.cpu_busy = None;
        assert_eq!(stall_of(&no_cpu, None), None);
    }
}
