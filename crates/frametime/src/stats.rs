//! Статистика кадрів і пошук піків frametime.
//!
//! Визначення (однакові всюди в програмі, щоб цифри до/після були порівнянні):
//! - **avg FPS** = кількість кадрів / сумарний час кадрів (а не середнє від миттєвих FPS);
//! - **1% low** = 1000 / середнє найповільніших 1% кадрів (так рахують CapFrameX і Afterburner);
//! - **0.1% low** — те саме для 0.1%;
//! - **пік** — кадр щонайменше вдвічі довший за локальну медіану (вікно 31 кадр) і довший
//!   за неї щонайменше на 4 мс, щоб не рахувати дрібні коливання на дуже високих FPS.

use crate::frame::FrameRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Розмір вікна локальної медіани для пошуку піків (непарний).
const SPIKE_WINDOW: usize = 31;
/// Пік — кадр, довший за локальну медіану щонайменше в стільки разів…
const SPIKE_RATIO: f64 = 2.0;
/// …і щонайменше на стільки мілісекунд.
const SPIKE_MIN_EXCESS_MS: f64 = 4.0;

/// Підсумкова статистика кадрів.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FrameStats {
    /// Кількість кадрів.
    pub frames: usize,
    /// Сумарна тривалість, мс.
    pub duration_ms: f64,
    /// Середній FPS.
    pub avg_fps: f64,
    /// 1% low, FPS.
    pub low_1pct_fps: f64,
    /// 0.1% low, FPS.
    pub low_01pct_fps: f64,
    /// Медіана frametime, мс.
    pub median_frametime_ms: f64,
    /// 99-й перцентиль frametime, мс.
    pub p99_frametime_ms: f64,
    /// Найдовший кадр, мс.
    pub max_frametime_ms: f64,
    /// Кількість піків.
    pub spikes: usize,
}

/// Пік frametime.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spike {
    /// Індекс кадру у вхідному масиві.
    pub index: usize,
    /// Тривалість кадру, мс.
    pub frametime_ms: f64,
    /// Локальна медіана навколо нього, мс.
    pub baseline_ms: f64,
}

fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn worst_mean(sorted_desc: &[f64], fraction: f64) -> f64 {
    let n = ((sorted_desc.len() as f64 * fraction).ceil() as usize).clamp(1, sorted_desc.len());
    sorted_desc[..n].iter().sum::<f64>() / n as f64
}

fn valid(frametimes_ms: &[f64]) -> Vec<f64> {
    frametimes_ms
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect()
}

/// Статистика за списком frametime у мілісекундах. `None`, якщо кадрів немає.
pub fn frame_stats(frametimes_ms: &[f64]) -> Option<FrameStats> {
    let ft = valid(frametimes_ms);
    if ft.is_empty() {
        return None;
    }
    let total: f64 = ft.iter().sum();
    let mut asc = ft.clone();
    asc.sort_by(f64::total_cmp);
    let desc: Vec<f64> = asc.iter().rev().copied().collect();
    let low1 = worst_mean(&desc, 0.01);
    let low01 = worst_mean(&desc, 0.001);
    Some(FrameStats {
        frames: ft.len(),
        duration_ms: total,
        avg_fps: ft.len() as f64 * 1000.0 / total,
        low_1pct_fps: 1000.0 / low1,
        low_01pct_fps: 1000.0 / low01,
        median_frametime_ms: percentile_sorted(&asc, 50.0),
        p99_frametime_ms: percentile_sorted(&asc, 99.0),
        max_frametime_ms: desc[0],
        spikes: find_spikes(&ft).len(),
    })
}

/// Шукає піки відносно локальної медіани (стійко до зміни сцени).
pub fn find_spikes(frametimes_ms: &[f64]) -> Vec<Spike> {
    let n = frametimes_ms.len();
    if n < 3 {
        return Vec::new();
    }
    let half = SPIKE_WINDOW / 2;
    let mut window: Vec<f64> = Vec::with_capacity(SPIKE_WINDOW);
    let mut out = Vec::new();
    for (i, &ft) in frametimes_ms.iter().enumerate() {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        window.clear();
        window.extend_from_slice(&frametimes_ms[lo..hi]);
        window.sort_by(f64::total_cmp);
        let median = window[window.len() / 2];
        if ft >= median * SPIKE_RATIO && ft - median >= SPIKE_MIN_EXCESS_MS {
            out.push(Spike {
                index: i,
                frametime_ms: ft,
                baseline_ms: median,
            });
        }
    }
    out
}

/// Процес і swap chain з найбільшою кількістю кадрів — це і є гра (а не лаунчер чи оверлей).
pub fn main_swap_chain(frames: &[FrameRecord]) -> Option<(u32, String)> {
    let mut counts: HashMap<(u32, &str), usize> = HashMap::new();
    for f in frames {
        *counts
            .entry((f.process_id, f.swap_chain.as_str()))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|((pid, sc), _)| (pid, sc.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameTimestamp;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn steady_100_fps() {
        let s = frame_stats(&[10.0; 1000]).unwrap();
        assert!(approx(s.avg_fps, 100.0));
        assert!(approx(s.low_1pct_fps, 100.0));
        assert!(approx(s.low_01pct_fps, 100.0));
        assert_eq!(s.spikes, 0);
        assert_eq!(s.frames, 1000);
    }

    #[test]
    fn lows_reflect_worst_frames() {
        // 990 кадрів по 10 мс і 10 кадрів по 50 мс: 1% = 10 найгірших = 50 мс → 20 FPS.
        let mut ft = vec![10.0; 990];
        ft.extend(std::iter::repeat_n(50.0, 10));
        let s = frame_stats(&ft).unwrap();
        assert!(approx(s.low_1pct_fps, 20.0));
        assert!(approx(s.low_01pct_fps, 20.0));
        // avg FPS = 1000 кадрів / (9900 + 500) мс.
        assert!(approx(s.avg_fps, 1000.0 * 1000.0 / 10_400.0));
        assert!(approx(s.max_frametime_ms, 50.0));
        assert!(approx(s.median_frametime_ms, 10.0));
    }

    #[test]
    fn invalid_values_are_ignored() {
        assert!(frame_stats(&[]).is_none());
        assert!(frame_stats(&[0.0, -1.0, f64::NAN]).is_none());
        assert_eq!(frame_stats(&[10.0, f64::INFINITY]).unwrap().frames, 1);
    }

    #[test]
    fn spikes_use_local_baseline() {
        let mut ft = vec![5.0; 200];
        ft[50] = 30.0; // пік
        ft[120] = 8.0; // коливання, не пік (лише +3 мс)
                       // Зміна сцени: стабільно 20 мс — не пік, бо локальна медіана теж 20.
        for v in ft.iter_mut().skip(150) {
            *v = 20.0;
        }
        let spikes = find_spikes(&ft);
        assert_eq!(spikes.len(), 1);
        assert_eq!(spikes[0].index, 50);
        assert!(approx(spikes[0].baseline_ms, 5.0));
    }

    #[test]
    fn high_fps_jitter_is_not_a_spike() {
        // 500 FPS: 2 мс → 4.5 мс — вдвічі більше, але лише +2.5 мс.
        let mut ft = vec![2.0; 100];
        ft[40] = 4.5;
        assert!(find_spikes(&ft).is_empty());
    }

    #[test]
    fn picks_swap_chain_with_most_frames() {
        let f = |pid, sc: &str| FrameRecord {
            application: "x".into(),
            process_id: pid,
            swap_chain: sc.into(),
            present_mode: String::new(),
            time: FrameTimestamp::Qpc(0),
            ms_between_presents: 1.0,
            ms_between_display_change: None,
            ms_cpu_busy: None,
            ms_gpu_busy: None,
            sync_interval: None,
            allows_tearing: None,
        };
        let frames = vec![f(1, "a"), f(2, "b"), f(2, "b"), f(2, "c")];
        assert_eq!(main_swap_chain(&frames), Some((2, "b".into())));
        assert_eq!(main_swap_chain(&[]), None);
    }
}
