//! Затримка кадру й вводу зі стовпців PresentMon 2.x (див. `optimax_frametime::FramePipeline`).
//!
//! Є три різні виміри, і вони не взаємозамінні:
//! - **Кадр → екран** — `MsCPUBusy + MsUntilDisplayed`: від початку кадру на CPU до показу. Є для
//!   кожного показаного кадру, навіть без вводу. Час від натискання до початку кадру сюди не
//!   входить, тож це нижня межа затримки.
//! - **Ввід → екран** і **клік → екран** — PresentMon сам пов'язує ввід із кадром; значення є лише
//!   в кадрах, на які вплинув ввід, який бачить Windows.
//! - **PC Latency** — лише в іграх, що передають розмітку PC Latency (Reflex).
//!
//! Жоден вимір не містить затримки миші, USB і самого монітора: без апаратного датчика їх не
//! виміряти. Тут лише опис вимірів; висновків і порогів немає.

use crate::timeline::FramePoint;
use serde::{Deserialize, Serialize};

/// Менше значень — статистику не показуємо (випадкові кілька кліків нічого не доводять).
pub const MIN_SAMPLES: usize = 10;

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
    /// Медіана: CPU готує кадр, мс.
    pub cpu_busy_ms: Option<f64>,
    /// Медіана: від початку кадру до початку роботи GPU над ним, мс (перекривається з CPU).
    pub gpu_start_ms: Option<f64>,
    /// Медіана: від Present до показу, мс.
    pub until_displayed_ms: Option<f64>,
    /// Кадрів у записі.
    pub frames: u32,
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

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_frametime::FramePipeline;

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
