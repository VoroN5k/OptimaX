//! Шкала запису для інтерактивного перегляду: кадри, частини кадру, потоки гри, завантаження
//! заліза й драйвери на одній осі часу.
//!
//! Тут лише перетворення виміряних даних у компактний вигляд для інтерфейсу (стовпці, округлення
//! до 0,01 мс); висновків і порогів тут немає — вони в [`crate::rules`] і [`crate::anatomy`].

use crate::rules::Analysis;
use serde::{Deserialize, Serialize};

/// Розрив між сусідніми кадрами, з якого шкала вважається перерваною (Alt+Tab, меню), мс.
const GAP_MS: f64 = 100.0;

/// Кадри головного swap chain, стовпцями (індекс — номер кадру).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FrameColumns {
    /// Час Present, мс від старту запису.
    pub t: Vec<f64>,
    /// Тривалість кадру (між Present), мс.
    pub ft: Vec<f32>,
    /// CPU готував кадр, мс.
    pub cpu: Vec<Option<f32>>,
    /// GPU працював над кадром, мс.
    pub gpu: Vec<Option<f32>>,
    /// Від початку кадру на CPU до початку роботи GPU (черга до відеокарти), мс.
    pub queue: Vec<Option<f32>>,
}

/// Знімки метрик, стовпцями.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SampleColumns {
    /// Час знімка, мс від старту запису.
    pub t: Vec<f64>,
    /// Завантаження відеокарти, %.
    pub gpu: Vec<Option<f32>>,
    /// Найзавантаженіше ядро, %.
    pub core: Vec<f32>,
}

/// Стани одного потоку гри: кошики за весь запис і точні проміжки навколо ривків.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThreadLane {
    /// Потік.
    pub tid: u32,
    /// Початок кошика 0, мс від старту запису.
    pub start_ms: f64,
    /// Ширина кошика, мс.
    pub bucket_ms: f64,
    /// Мс у кошику за станами: працює / чекає на ядро / синхронізація / диск / сон / інше.
    pub buckets: Vec<[f32; 6]>,
    /// Точні проміжки: (початок, мс від старту запису; тривалість, мс; стан — як у кошиках).
    pub detail: Vec<(f64, f32, u8)>,
}

/// Довга подія DPC/ISR драйвера.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DpcMark {
    /// Початок, мс від старту запису.
    pub t: f64,
    /// Тривалість, мкс.
    pub us: f32,
    /// Індекс у [`TimelineView::drivers`].
    pub driver: u16,
    /// Обробник переривання (а не DPC).
    pub isr: bool,
}

/// Усе, що виміряно за запис, на одній шкалі часу.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TimelineView {
    /// Початок шкали — Present першого зарахованого кадру, мс від старту запису (як у радарі:
    /// від нього рахуються секунди в інтерфейсі).
    pub start_ms: f64,
    /// Кінець шкали (Present останнього кадру), мс.
    pub end_ms: f64,
    /// Медіана frametime, мс.
    pub median_ms: f64,
    /// Кадри.
    pub frames: FrameColumns,
    /// Ривки: (час Present, мс; тривалість кадру, мс), за часом.
    pub spikes: Vec<(f64, f32)>,
    /// Відрізки без зарахованих кадрів: (від, до), мс.
    pub gaps: Vec<(f64, f64)>,
    /// Завантаження заліза.
    pub samples: SampleColumns,
    /// Головний потік гри (трасування потоків).
    pub main: Option<ThreadLane>,
    /// Рендер-потік (Present), якщо це інший потік.
    pub render: Option<ThreadLane>,
    /// Довгі DPC/ISR драйверів.
    pub dpc: Vec<DpcMark>,
    /// Імена драйверів для [`DpcMark::driver`] (порожній рядок — модуль невідомий).
    pub drivers: Vec<String>,
    /// Записи в кеш шейдерів, мс від старту запису.
    pub shader_writes: Vec<f64>,
}

/// Округлення до 0,01 — менше тексту в JSON, точність вища за роздільність шкали.
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn r2f(v: f64) -> f32 {
    r2(v) as f32
}

fn lane(
    tid: u32,
    start_ms: f64,
    bucket_ms: f64,
    buckets: &[[f32; 6]],
    detail: &[(f64, f64, u8)],
) -> ThreadLane {
    ThreadLane {
        tid,
        start_ms: r2(start_ms),
        bucket_ms,
        buckets: buckets
            .iter()
            .map(|b| b.map(|v| r2f(f64::from(v))))
            .collect(),
        detail: detail.iter().map(|&(a, d, s)| (r2(a), r2f(d), s)).collect(),
    }
}

/// Будує шкалу з даних аналізу.
pub fn timeline_view(a: &Analysis<'_>) -> TimelineView {
    let tl = a.timeline;
    let f = &tl.frames;
    let frames = FrameColumns {
        t: f.iter().map(|p| r2(p.t_ms)).collect(),
        ft: f.iter().map(|p| r2f(p.frametime_ms)).collect(),
        cpu: f.iter().map(|p| p.cpu_busy_ms.map(r2f)).collect(),
        gpu: f.iter().map(|p| p.gpu_busy_ms.map(r2f)).collect(),
        queue: f.iter().map(|p| p.pipeline.gpu_latency.map(r2f)).collect(),
    };
    let gaps = f
        .windows(2)
        .filter(|w| w[1].t_ms - w[1].frametime_ms - w[0].t_ms > GAP_MS)
        .map(|w| (r2(w[0].t_ms), r2(w[1].t_ms - w[1].frametime_ms)))
        .collect();
    let samples = SampleColumns {
        t: tl.samples.iter().map(|s| r2(s.t_ms)).collect(),
        gpu: tl
            .samples
            .iter()
            .map(|s| s.gpu.as_ref().and_then(|g| g.util).map(|u| u as f32))
            .collect(),
        core: tl.samples.iter().map(|s| s.busiest_core_pct).collect(),
    };
    let (main, render) = tl.threads.as_ref().map_or((None, None), |t| {
        let main = lane(
            t.main.tid,
            t.buckets_start_ms,
            t.bucket_ms,
            &t.main_buckets,
            &t.main_detail,
        );
        let render = t.render.map(|r| {
            lane(
                r.tid,
                t.buckets_start_ms,
                t.bucket_ms,
                &t.render_buckets,
                &t.render_detail,
            )
        });
        (Some(main), render)
    });
    let (dpc, drivers) = tl.dpc.as_ref().map_or((Vec::new(), Vec::new()), |d| {
        let marks = d
            .long
            .iter()
            .map(|e| DpcMark {
                t: r2(e.t_ms),
                us: r2f(e.dur_us),
                driver: e.driver,
                isr: e.isr,
            })
            .collect();
        let names = d
            .drivers
            .iter()
            .map(|x| x.name.clone().unwrap_or_default())
            .collect();
        (marks, names)
    });
    TimelineView {
        start_ms: f.first().map_or(0.0, |p| r2(p.t_ms)),
        end_ms: f.last().map_or(0.0, |p| r2(p.t_ms)),
        median_ms: a.stats.median_frametime_ms,
        frames,
        spikes: a
            .spikes
            .iter()
            .map(|&(t, ms, _)| (r2(t), r2f(ms)))
            .collect(),
        gaps,
        samples,
        main,
        render,
        dpc,
        drivers,
        shader_writes: tl.shader_writes.iter().map(|w| r2(w.t_ms)).collect(),
    }
}
