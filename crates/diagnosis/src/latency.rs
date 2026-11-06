//! Затримка кадру й вводу зі стовпців PresentMon 2.x (див. `optimax_frametime::FramePipeline`).
//!
//! Є три різні виміри, і вони не взаємозамінні:
//! - **Кадр → екран** — `MsCPUBusy + MsUntilDisplayed`: від початку кадру на CPU до показу. Є для
//!   кожного показаного кадру, навіть без вводу. Час від натискання до початку кадру сюди не
//!   входить, тож це нижня межа затримки.
//! - **PC Latency** — лише в іграх, що передають розмітку PC Latency (Reflex): згладжений час від
//!   вводу до початку кадру + від початку кадру до показу (`CalculatePcLatency`, PresentMon 2.6.0).
//!   Найповніший вимір.
//! - **Ввід → екран** і **клік → екран** — лише довідково. За кодом 2.6.0 відлік іде від останнього
//!   вводу, який Windows прочитала, коли гра забрала повідомлення вводу, а не від вводу, який гра
//!   врахувала в кадрі. У RE9: ввід → екран 7,9 мс при кадрі → екран 14,1 мс, тож вимір занижує
//!   затримку; вердикти на ньому не будувати.
//!
//! Жоден вимір не містить затримки миші, USB і самого монітора: без апаратного датчика їх не
//! виміряти.
//!
//! Черга перед початком кадру видна лише в PC Latency: у RE9 з GPU у стелі Reflex On + Boost знизив
//! її з 47,1 до 32,9 мс, а кадр → екран лишився 27,9 мс. Звідси єдиний висновок модуля —
//! [`reflex_hint`]; це підказка, а не обіцянка: дані — по одному запису на варіант в одній грі.

use crate::timeline::FramePoint;
use optimax_core::{Category, Finding, FindingBuilder, Impact, Text};
use serde::{Deserialize, Serialize};

/// Менше значень — статистику не показуємо (випадкові кілька кліків нічого не доводять).
pub const MIN_SAMPLES: usize = 10;
/// З якого надлишку PC Latency над кадром → екран (у кадрах) є сенс пропонувати Reflex. У RE9 з GPU
/// у стелі: 1,4 кадра без Reflex, 0,35 — з ним.
pub const QUEUE_GAP_FRAMES: f64 = 1.0;
/// PC Latency порівнюємо з кадром → екран лише якщо вона є щонайменше в такій частці кадрів.
const PC_LATENCY_MIN_COVERAGE: f64 = 0.5;
/// Впевненість підказки: ознака не доводить чергу (при ≈134 FPS надлишок 1,2 кадра Reflex не
/// прибрав), а виміряно її лише в одній грі.
const REFLEX_HINT_CONFIDENCE: u8 = 60;

/// Розподіл одного виміру затримки, мс.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LatencyStat {
    /// Медіана.
    pub median_ms: f64,
    /// 95-й перцентиль.
    pub p95_ms: f64,
    /// Кількість кадрів зі значенням.
    pub samples: u32,
}

/// Затримка за запис.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LatencySummary {
    /// Від початку кадру на CPU до показу на екрані.
    pub frame_to_display: Option<LatencyStat>,
    /// Від вводу (клавіатура чи миша) до показу.
    pub input_to_photon: Option<LatencyStat>,
    /// Від кліку миші до показу.
    pub click_to_photon: Option<LatencyStat>,
    /// PC Latency (гра передає розмітку).
    pub pc_latency: Option<LatencyStat>,
    /// Медіана `MsCPUBusy`: від початку кадру до Present, мс. Імовірно, містить і очікування GPU:
    /// у RE9 без апскейлу значення подвоїлося (7,1 → 13,7 мс).
    pub cpu_busy_ms: Option<f64>,
    /// Медіана: від початку кадру до початку роботи GPU над ним, мс (перекривається з CPU).
    pub gpu_start_ms: Option<f64>,
    /// Медіана: від Present до показу, мс.
    pub until_displayed_ms: Option<f64>,
    /// Кадрів у записі.
    pub frames: u32,
}

impl LatencySummary {
    /// PC Latency, якщо вона є в більшості кадрів: лише тоді значення описує запис загалом.
    pub fn reliable_pc_latency(&self) -> Option<LatencyStat> {
        self.pc_latency
            .filter(|p| f64::from(p.samples) >= f64::from(self.frames) * PC_LATENCY_MIN_COVERAGE)
    }
}

/// Перцентиль методом найближчого рангу (`sorted` не порожній).
fn nearest_rank(sorted: &[f64], p: f64) -> f64 {
    let rank = (sorted.len() as f64 * p).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Статистика додатних значень. `None` — менше [`MIN_SAMPLES`].
fn stat(values: impl Iterator<Item = f64>) -> Option<LatencyStat> {
    let mut v: Vec<f64> = values.filter(|x| x.is_finite() && *x > 0.0).collect();
    if v.len() < MIN_SAMPLES {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(LatencyStat {
        median_ms: nearest_rank(&v, 0.5),
        p95_ms: nearest_rank(&v, 0.95),
        samples: u32::try_from(v.len()).unwrap_or(u32::MAX),
    })
}

/// Зведення затримки. `None` — PresentMon не дав жодного з вимірів (стара версія, 1.x).
pub fn latency_summary(frames: &[FramePoint]) -> Option<LatencySummary> {
    let frame_to_display = stat(frames.iter().filter_map(|f| {
        // Непоказаний кадр не має `MsUntilDisplayed`, тож у вимір не потрапляє.
        Some(f.cpu_busy_ms? + f.pipeline.until_displayed.filter(|v| *v > 0.0)?)
    }));
    let input_to_photon = stat(frames.iter().filter_map(|f| f.pipeline.input_to_photon));
    let click_to_photon = stat(frames.iter().filter_map(|f| f.pipeline.click_to_photon));
    let pc_latency = stat(frames.iter().filter_map(|f| f.pipeline.pc_latency));
    if frame_to_display.is_none()
        && input_to_photon.is_none()
        && click_to_photon.is_none()
        && pc_latency.is_none()
    {
        return None;
    }
    let med = |get: fn(&FramePoint) -> Option<f64>| {
        stat(frames.iter().filter_map(get)).map(|s| s.median_ms)
    };
    Some(LatencySummary {
        frame_to_display,
        input_to_photon,
        click_to_photon,
        pc_latency,
        cpu_busy_ms: med(|f| f.cpu_busy_ms),
        gpu_start_ms: med(|f| f.pipeline.gpu_latency),
        until_displayed_ms: med(|f| f.pipeline.until_displayed),
        frames: u32::try_from(frames.len()).unwrap_or(u32::MAX),
    })
}

/// Підказка увімкнути Reflex: PC Latency більша за кадр → екран щонайменше на
/// [`QUEUE_GAP_FRAMES`]. Викликати лише коли обмежує відеокарта: в іншому разі черги, яку прибирає
/// Reflex, може не бути. `frame_ms` — медіана часу кадру.
pub fn reflex_hint(s: &LatencySummary, frame_ms: f64) -> Option<Finding> {
    let pcl = s.reliable_pc_latency()?;
    let display = s.frame_to_display?;
    if frame_ms <= 0.0 {
        return None;
    }
    let gap = pcl.median_ms - display.median_ms;
    let frames = gap / frame_ms;
    if frames < QUEUE_GAP_FRAMES {
        return None;
    }
    let value = |v: f64| Text::new("value.ms").arg("v", format!("{v:.1}"));
    Some(
        FindingBuilder::new(
            "diag",
            "latency_queue",
            Category::Gpu,
            Impact::Info,
            REFLEX_HINT_CONFIDENCE,
        )
        .arg("gap", format!("{gap:.1}"))
        .arg("frames", format!("{frames:.1}"))
        .advice()
        .evidence(Text::new("diag.ev.pc_latency"), value(pcl.median_ms))
        .evidence(
            Text::new("diag.ev.frame_to_display"),
            value(display.median_ms),
        )
        .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_frametime::FramePipeline;

    /// Записи RE9 (2026-11-06): PC Latency, кадр → екран, час кадру.
    fn re9(pcl: f64, display: f64, frames: u32) -> LatencySummary {
        let s = |median_ms| LatencyStat {
            median_ms,
            p95_ms: median_ms,
            samples: frames,
        };
        LatencySummary {
            frame_to_display: Some(s(display)),
            input_to_photon: None,
            click_to_photon: None,
            pc_latency: Some(s(pcl)),
            cpu_busy_ms: None,
            gpu_start_ms: None,
            until_displayed_ms: None,
            frames,
        }
    }

    #[test]
    fn reflex_hint_fires_without_reflex_and_stays_quiet_with_it() {
        // GPU у стелі, без Reflex: надлишок 19,2 мс ≈ 1,4 кадра.
        let f = reflex_hint(&re9(47.1, 27.9, 4303), 14.1).expect("підказка");
        assert_eq!(f.id, "latency_queue");
        assert_eq!(f.title.args["gap"], "19.2");
        assert_eq!(f.title.args["frames"], "1.4");
        assert!(f.advice.is_some());
        assert_eq!(f.evidence.len(), 2);
        // Те саме з Reflex On + Boost: 5,0 мс ≈ 0,35 кадра.
        assert!(reflex_hint(&re9(32.9, 27.9, 4230), 14.2).is_none());
    }

    #[test]
    fn reflex_hint_needs_pc_latency_in_most_frames() {
        let mut s = re9(47.1, 27.9, 4303);
        s.pc_latency = None;
        assert!(
            reflex_hint(&s, 14.1).is_none(),
            "без PC Latency чергу не видно"
        );
        let mut s = re9(47.1, 27.9, 4303);
        if let Some(p) = s.pc_latency.as_mut() {
            p.samples = 2000;
        }
        assert!(
            reflex_hint(&s, 14.1).is_none(),
            "значення лише в частині кадрів"
        );
        assert!(reflex_hint(&re9(47.1, 27.9, 4303), 0.0).is_none());
    }

    fn frame(i: usize) -> FramePoint {
        FramePoint {
            t_ms: i as f64 * 10.0,
            frametime_ms: 10.0,
            display_ms: None,
            cpu_busy_ms: Some(5.0 + (i % 10) as f64),
            gpu_busy_ms: None,
            present_mode: optimax_frametime::PresentMode::Other,
            sync_interval: None,
            allows_tearing: None,
            pipeline: FramePipeline {
                until_displayed: Some(10.0),
                gpu_latency: Some(3.0),
                ..Default::default()
            },
        }
    }

    #[test]
    fn frame_to_display_is_cpu_work_plus_present_to_display() {
        let frames: Vec<FramePoint> = (0..100).map(frame).collect();
        let s = latency_summary(&frames).expect("зведення");
        let d = s.frame_to_display.expect("кадр → екран");
        assert_eq!(d.samples, 100);
        // CPU 5..14 мс + 10 мс до екрана.
        assert_eq!(d.median_ms, 19.0);
        assert_eq!(d.p95_ms, 24.0);
        assert_eq!(s.cpu_busy_ms, Some(9.0));
        assert_eq!(s.gpu_start_ms, Some(3.0));
        assert_eq!(s.until_displayed_ms, Some(10.0));
        assert_eq!(s.frames, 100);
        assert!(s.input_to_photon.is_none() && s.pc_latency.is_none());
    }

    #[test]
    fn dropped_frames_are_left_out() {
        let mut frames: Vec<FramePoint> = (0..100).map(frame).collect();
        for f in frames.iter_mut().take(40) {
            f.pipeline.until_displayed = None;
        }
        // PresentMon пише NA для 0, але старі записи можуть мати нуль.
        frames[50].pipeline.until_displayed = Some(0.0);
        let s = latency_summary(&frames).expect("зведення");
        assert_eq!(s.frame_to_display.expect("кадр → екран").samples, 59);
    }

    #[test]
    fn a_few_clicks_prove_nothing() {
        let mut frames: Vec<FramePoint> = (0..100).map(frame).collect();
        for f in frames.iter_mut().take(MIN_SAMPLES - 1) {
            f.pipeline.click_to_photon = Some(30.0);
        }
        for f in frames.iter_mut().take(MIN_SAMPLES) {
            f.pipeline.input_to_photon = Some(25.0);
        }
        let s = latency_summary(&frames).expect("зведення");
        assert!(s.click_to_photon.is_none());
        let i = s.input_to_photon.expect("ввід");
        assert_eq!((i.samples, i.median_ms), (MIN_SAMPLES as u32, 25.0));
    }

    #[test]
    fn no_columns_no_summary() {
        let frames: Vec<FramePoint> = (0..100)
            .map(|i| {
                let mut f = frame(i);
                f.cpu_busy_ms = None;
                f.pipeline = FramePipeline::default();
                f
            })
            .collect();
        assert!(latency_summary(&frames).is_none());
        // Лише PC Latency — теж зведення.
        let mut with_pcl = frames;
        for f in &mut with_pcl {
            f.pipeline.pc_latency = Some(22.0);
        }
        let s = latency_summary(&with_pcl).expect("зведення");
        assert_eq!(s.pc_latency.expect("PCL").median_ms, 22.0);
        assert!(s.frame_to_display.is_none());
    }
}
