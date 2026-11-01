//! Радар мікрофризів: стиснутий графік frametime і позначки ривків із ймовірними причинами.
//!
//! Графік стискається до [`CHART_POINTS`] відрізків часу; у кожному зберігається найдовший
//! кадр, тож жоден ривок не губиться при стисненні. Позначок — не більше [`MAX_MARKS`]
//! (найдовші ривки), причини кожної — ті самі ознаки, що й у вердикті мікрофризів
//! ([`crate::rules::CauseKind`]), але перевірені саме біля цього ривка.

use crate::rules::{Analysis, CauseContext, CauseKind, ABOVE_USUAL};
use optimax_core::Text;
use serde::{Deserialize, Serialize};

/// Скільки точок у графіку.
pub const CHART_POINTS: usize = 600;
/// Скільки ривків позначати (найдовші).
pub const MAX_MARKS: usize = 200;

/// Точка графіка: відрізок часу.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChartPoint {
    /// Початок відрізка, мс від старту запису.
    pub t_ms: f64,
    /// Найдовший кадр у відрізку, мс.
    pub max_ms: f32,
    /// Середній кадр у відрізку, мс.
    pub avg_ms: f32,
}

/// Ривок на графіку.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SpikeMark {
    /// Час кадру, мс від старту запису.
    pub t_ms: f64,
    /// Тривалість кадру, мс.
    pub frametime_ms: f64,
    /// Звичайний кадр поруч (локальна медіана), мс.
    pub baseline_ms: f64,
    /// Що було поруч (порожньо — причину не знайдено).
    pub causes: Vec<Text>,
    /// Коди ознак поруч (як [`CauseCount::code`]).
    #[serde(default)]
    pub codes: Vec<String>,
}

/// Скільки позначених ривків мали поруч цю ознаку.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CauseCount {
    /// Код ознаки.
    pub code: String,
    /// Коротка назва ознаки.
    pub name: Text,
    /// Кількість позначених ривків з нею поруч.
    pub spikes: u32,
    /// Частка звичайних моментів запису з цією ознакою (0–1).
    #[serde(default)]
    pub usual: f64,
    /// Біля ривків ознака трапляється помітно частіше, ніж зазвичай (інакше це тло).
    #[serde(default)]
    pub above_usual: bool,
}

/// Графік frametime для «Чому лагає?».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FrametimeChart {
    /// Початок шкали — перший зарахований кадр, мс від старту запису (до нього —
    /// очікування гри й прогрів, їх не показуємо).
    pub start_ms: f64,
    /// Кінець шкали, мс.
    pub duration_ms: f64,
    /// Медіана frametime — «звичайний» кадр, мс.
    pub median_ms: f64,
    /// Точки графіка за часом.
    pub points: Vec<ChartPoint>,
    /// Позначені ривки за часом.
    pub spikes: Vec<SpikeMark>,
    /// Скільки ривків усього (позначено не більше [`MAX_MARKS`]).
    pub spikes_total: u32,
    /// Зведення по позначених ривках: кожна ознака й скільки ривків мали її поруч (один
    /// ривок може мати кілька ознак), від найчастішої.
    #[serde(default)]
    pub summary: Vec<CauseCount>,
    /// Позначених ривків без жодної виміряної ознаки поруч.
    #[serde(default)]
    pub unexplained: u32,
}

/// Стискає кадри до `n` відрізків часу від першого до останнього кадру (порожні відрізки
/// пропускаються).
pub fn downsample(frames: &[(f64, f64)], n: usize) -> Vec<ChartPoint> {
    let (Some(&(first, _)), Some(&(last, _))) = (frames.first(), frames.last()) else {
        return Vec::new();
    };
    let n = n.max(1);
    let width = ((last - first) / n as f64).max(f64::EPSILON);
    let mut out: Vec<ChartPoint> = Vec::with_capacity(n);
    let mut bucket = usize::MAX;
    let (mut sum, mut count) = (0.0, 0u32);
    for &(t, ft) in frames {
        let b = (((t - first) / width) as usize).min(n - 1);
        if b != bucket {
            if let Some(p) = out.last_mut().filter(|_| count > 0) {
                p.avg_ms = (sum / f64::from(count)) as f32;
            }
            out.push(ChartPoint {
                t_ms: first + b as f64 * width,
                max_ms: ft as f32,
                avg_ms: ft as f32,
            });
            bucket = b;
            sum = 0.0;
            count = 0;
        }
        if let Some(p) = out.last_mut() {
            p.max_ms = p.max_ms.max(ft as f32);
        }
        sum += ft;
        count += 1;
    }
    if let Some(p) = out.last_mut().filter(|_| count > 0) {
        p.avg_ms = (sum / f64::from(count)) as f32;
    }
    out
}

/// Будує графік із позначками ривків.
pub fn frametime_chart(a: &Analysis<'_>) -> FrametimeChart {
    let frames: Vec<(f64, f64)> = a
        .timeline
        .frames
        .iter()
        .map(|f| (f.t_ms, f.frametime_ms))
        .collect();
    let mut sorted: Vec<f64> = frames.iter().map(|f| f.1).collect();
    sorted.sort_by(f64::total_cmp);
    let median_ms = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);

    // Найдовші ривки, далі — за часом.
    let mut worst: Vec<(f64, f64, f64)> = a.spikes.clone();
    worst.sort_by(|x, y| y.1.total_cmp(&x.1));
    worst.truncate(MAX_MARKS);
    worst.sort_by(|x, y| x.0.total_cmp(&y.0));

    let ctx = CauseContext::new(a);
    let mut counts = [0u32; CauseKind::ALL.len()];
    let mut unexplained = 0u32;
    let spikes = worst
        .into_iter()
        .map(|(t, ft, base)| {
            let mut causes = Vec::new();
            let mut codes = Vec::new();
            for (i, k) in CauseKind::ALL.into_iter().enumerate() {
                if ctx.hit_near(a, k, t) {
                    counts[i] += 1;
                    causes.push(ctx.detail(a, k, t));
                    codes.push(k.code().to_string());
                }
            }
            if causes.is_empty() {
                unexplained += 1;
            }
            SpikeMark {
                t_ms: t,
                frametime_ms: ft,
                baseline_ms: base,
                causes,
                codes,
            }
        })
        .collect();
    let mut summary: Vec<CauseCount> = CauseKind::ALL
        .into_iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| {
            let (at_spikes, usual) = ctx.coincidence(a, k);
            CauseCount {
                code: k.code().to_string(),
                name: Text::new(k.name_key()),
                spikes: n,
                usual,
                above_usual: at_spikes >= usual * ABOVE_USUAL,
            }
        })
        .collect();
    summary.sort_by_key(|c| std::cmp::Reverse(c.spikes));

    FrametimeChart {
        start_ms: frames.first().map_or(0.0, |f| f.0),
        duration_ms: frames.last().map_or(0.0, |f| f.0),
        median_ms,
        points: downsample(&frames, CHART_POINTS),
        spikes,
        spikes_total: u32::try_from(a.spikes.len()).unwrap_or(u32::MAX),
        summary,
        unexplained,
    }
}
