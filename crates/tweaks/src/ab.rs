//! A/B-тест зміни: прогони чергуються ABAB…, рішення — лише за статистично значущою
//! і практично помітною різницею. Інакше «ефекту немає» і зміна відкочується (CLAUDE.md, розд. 6).
//!
//! Метод: для середнього FPS і 1% low окремо рахується різниця середніх B − A та її 95 %
//! довірчий інтервал за t-тестом Велча (дисперсії груп можуть відрізнятися). Ефект є, якщо
//! інтервал не перетинає нуль **і** різниця не менша за [`MIN_EFFECT_PCT`].

use crate::TweakError;
use optimax_core::{Catalog, Text};
use serde::{Deserialize, Serialize};

/// Мінімум прогонів на кожен варіант.
pub const MIN_RUNS_PER_ARM: usize = 3;
/// Менша різниця вважається неістотною навіть за статистичної значущості, %.
pub const MIN_EFFECT_PCT: f64 = 1.0;
/// Різниця середніх температур GPU між варіантами, після якої умови вважаються різними, °C.
const TEMP_DRIFT_C: f64 = 5.0;
/// Коефіцієнт варіації FPS усередині варіанта, після якого прогони нестабільні, %.
const UNSTABLE_CV_PCT: f64 = 5.0;
/// Розбіжність тривалостей прогонів, після якої варто попередити, частка.
const DURATION_MISMATCH: f64 = 0.10;
/// Менше кадрів у прогоні — замало для надійного 1% low.
const MIN_FRAMES_PER_RUN: usize = 1000;

/// Варіант: A — як було, B — зі зміною.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arm {
    /// Без зміни.
    A,
    /// Зі зміною.
    B,
}

/// Порядок прогонів: A, B, A, B… (`runs_per_arm` кожного, не менше трьох).
pub fn schedule(runs_per_arm: usize) -> Vec<Arm> {
    let n = runs_per_arm.max(MIN_RUNS_PER_ARM);
    (0..n).flat_map(|_| [Arm::A, Arm::B]).collect()
}

/// Умови прогону — щоб помітити, якщо вони відрізнялися.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConditions {
    /// Тривалість, с.
    pub duration_s: f64,
    /// Кадрів.
    pub frames: usize,
    /// Середня температура GPU, °C.
    pub gpu_temp_avg_c: Option<f64>,
    /// FPS упирався в ліміт кадрів (зміна не може проявитись у FPS).
    pub frame_capped: bool,
}

/// Результат одного прогону.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    /// Варіант.
    pub arm: Arm,
    /// Середній FPS.
    pub avg_fps: f64,
    /// 1% low.
    pub low_1pct_fps: f64,
    /// Умови.
    pub conditions: RunConditions,
}

/// Описова статистика одного варіанта.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Середнє.
    pub mean: f64,
    /// Стандартне відхилення (вибіркове).
    pub sd: f64,
    /// Кількість прогонів.
    pub n: usize,
}

impl Summary {
    fn of(values: &[f64]) -> Self {
        let n = values.len();
        let mean = values.iter().sum::<f64>() / n.max(1) as f64;
        let var = if n > 1 {
            values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64
        } else {
            0.0
        };
        Self {
            mean,
            sd: var.sqrt(),
            n,
        }
    }

    /// Коефіцієнт варіації, %.
    pub fn cv_pct(&self) -> f64 {
        if self.mean > 0.0 {
            self.sd / self.mean * 100.0
        } else {
            0.0
        }
    }
}

/// Висновок для однієї метрики.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Стало краще.
    Better,
    /// Стало гірше.
    Worse,
    /// Різниця в межах похибки або менша за поріг.
    NoEffect,
}

/// Порівняння варіантів за однією метрикою.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// Без зміни.
    pub a: Summary,
    /// Зі зміною.
    pub b: Summary,
    /// Різниця B − A, % від A.
    pub delta_pct: f64,
    /// Нижня межа 95 % інтервалу, %.
    pub ci_low_pct: f64,
    /// Верхня межа 95 % інтервалу, %.
    pub ci_high_pct: f64,
    /// Висновок.
    pub outcome: Outcome,
}

/// Рішення щодо зміни.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Залишити.
    Keep,
    /// Відкотити.
    Revert,
}

/// Звіт A/B.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbReport {
    /// Середній FPS.
    pub avg_fps: Comparison,
    /// 1% low.
    pub low_1pct: Comparison,
    /// Загальний висновок.
    pub effect: Outcome,
    /// Що робити зі зміною.
    pub decision: Decision,
    /// Попередження про умови тесту.
    pub warnings: Vec<Text>,
    /// Усі прогони в порядку виконання.
    pub runs: Vec<RunResult>,
}

/// Критичне значення t для двобічних 95 % за кількістю ступенів свободи.
fn t_critical(df: f64) -> f64 {
    const TABLE: [(f64, f64); 15] = [
        (1.0, 12.706),
        (2.0, 4.303),
        (3.0, 3.182),
        (4.0, 2.776),
        (5.0, 2.571),
        (6.0, 2.447),
        (7.0, 2.365),
        (8.0, 2.306),
        (9.0, 2.262),
        (10.0, 2.228),
        (12.0, 2.179),
        (15.0, 2.131),
        (20.0, 2.086),
        (30.0, 2.042),
        (60.0, 2.000),
    ];
    if !df.is_finite() || df > 60.0 {
        return 1.96;
    }
    // Беремо найближче менше df — консервативно (ширший інтервал).
    TABLE
        .iter()
        .rev()
        .find(|(d, _)| *d <= df.max(1.0))
        .map_or(TABLE[0].1, |(_, t)| *t)
}

fn compare(a: &[f64], b: &[f64]) -> Comparison {
    let (sa, sb) = (Summary::of(a), Summary::of(b));
    let delta = sb.mean - sa.mean;
    let va = sa.sd.powi(2) / sa.n as f64;
    let vb = sb.sd.powi(2) / sb.n as f64;
    let se = (va + vb).sqrt();
    let df = if se > 0.0 {
        (va + vb).powi(2) / (va.powi(2) / (sa.n - 1) as f64 + vb.powi(2) / (sb.n - 1) as f64)
    } else {
        f64::INFINITY
    };
    let half = t_critical(df) * se;
    let to_pct = |v: f64| {
        if sa.mean > 0.0 {
            v / sa.mean * 100.0
        } else {
            0.0
        }
    };
    let (delta_pct, lo, hi) = (to_pct(delta), to_pct(delta - half), to_pct(delta + half));
    let outcome = if lo > 0.0 && delta_pct >= MIN_EFFECT_PCT {
        Outcome::Better
    } else if hi < 0.0 && delta_pct <= -MIN_EFFECT_PCT {
        Outcome::Worse
    } else {
        Outcome::NoEffect
    };
    Comparison {
        a: sa,
        b: sb,
        delta_pct,
        ci_low_pct: lo,
        ci_high_pct: hi,
        outcome,
    }
}

fn mean_of(values: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = values.collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn warnings(runs: &[RunResult], avg: &Comparison) -> Vec<Text> {
    let mut out = Vec::new();
    let temp = |arm| {
        mean_of(
            runs.iter()
                .filter(|r| r.arm == arm)
                .filter_map(|r| r.conditions.gpu_temp_avg_c),
        )
    };
    if let (Some(ta), Some(tb)) = (temp(Arm::A), temp(Arm::B)) {
        if (ta - tb).abs() > TEMP_DRIFT_C {
            out.push(
                Text::new("ab.warn.temp_drift")
                    .arg("a", format!("{ta:.0}"))
                    .arg("b", format!("{tb:.0}")),
            );
        }
    }
    let durations: Vec<f64> = runs.iter().map(|r| r.conditions.duration_s).collect();
    let (dmin, dmax) = durations
        .iter()
        .fold((f64::MAX, 0.0f64), |(lo, hi), d| (lo.min(*d), hi.max(*d)));
    if dmax > 0.0 && (dmax - dmin) / dmax > DURATION_MISMATCH {
        out.push(
            Text::new("ab.warn.duration_mismatch")
                .arg("min", format!("{dmin:.0}"))
                .arg("max", format!("{dmax:.0}")),
        );
    }
    if runs.iter().any(|r| r.conditions.frame_capped) {
        out.push(Text::new("ab.warn.capped"));
    }
    for (s, name) in [(avg.a, "A"), (avg.b, "B")] {
        if s.cv_pct() > UNSTABLE_CV_PCT {
            out.push(
                Text::new("ab.warn.unstable")
                    .arg("arm", name)
                    .arg("cv", format!("{:.1}", s.cv_pct())),
            );
        }
    }
    if runs
        .iter()
        .any(|r| r.conditions.frames < MIN_FRAMES_PER_RUN)
    {
        out.push(Text::new("ab.warn.few_frames").arg("min", MIN_FRAMES_PER_RUN));
    }
    out
}

/// Оцінює прогони.
pub fn evaluate(runs: &[RunResult]) -> Result<AbReport, TweakError> {
    let pick = |arm: Arm, f: fn(&RunResult) -> f64| -> Vec<f64> {
        runs.iter().filter(|r| r.arm == arm).map(f).collect()
    };
    let (a_avg, b_avg) = (pick(Arm::A, |r| r.avg_fps), pick(Arm::B, |r| r.avg_fps));
    if a_avg.len() < MIN_RUNS_PER_ARM || b_avg.len() < MIN_RUNS_PER_ARM {
        return Err(TweakError::NotEnoughRuns {
            min: MIN_RUNS_PER_ARM,
            a: a_avg.len(),
            b: b_avg.len(),
        });
    }
    let avg_fps = compare(&a_avg, &b_avg);
    let low_1pct = compare(
        &pick(Arm::A, |r| r.low_1pct_fps),
        &pick(Arm::B, |r| r.low_1pct_fps),
    );
    let outcomes = [avg_fps.outcome, low_1pct.outcome];
    let effect = if outcomes.contains(&Outcome::Worse) {
        Outcome::Worse
    } else if outcomes.contains(&Outcome::Better) {
        Outcome::Better
    } else {
        Outcome::NoEffect
    };
    Ok(AbReport {
        warnings: warnings(runs, &avg_fps),
        avg_fps,
        low_1pct,
        decision: if effect == Outcome::Better {
            Decision::Keep
        } else {
            Decision::Revert
        },
        effect,
        runs: runs.to_vec(),
    })
}

fn signed(v: f64) -> String {
    if v >= 0.0 {
        format!("+{v:.1}")
    } else {
        format!("{v:.1}")
    }
}

fn row(c: &Catalog, label: &str, cmp: &Comparison) -> String {
    let cell = |s: &Summary| format!("{:.1} ± {:.1}", s.mean, s.sd);
    let change = c.render(
        &Text::new("ab.md.change")
            .arg("delta", signed(cmp.delta_pct))
            .arg("lo", signed(cmp.ci_low_pct))
            .arg("hi", signed(cmp.ci_high_pct)),
    );
    let outcome = c.render(&Text::new(match cmp.outcome {
        Outcome::Better => "ab.outcome.better",
        Outcome::Worse => "ab.outcome.worse",
        Outcome::NoEffect => "ab.outcome.no_effect",
    }));
    format!(
        "| {} | {} | {} | {change} | {outcome} |",
        c.render(&Text::new(label)),
        cell(&cmp.a),
        cell(&cmp.b)
    )
}

/// Звіт у Markdown — щоб можна було викласти на форумі.
pub fn render_markdown(report: &AbReport, c: &Catalog, tweak_title: &str, game: &str) -> String {
    let runs_per_arm = report.avg_fps.a.n;
    let duration = mean_of(report.runs.iter().map(|r| r.conditions.duration_s)).unwrap_or(0.0);
    let order: String = report
        .runs
        .iter()
        .map(|r| match r.arm {
            Arm::A => 'A',
            Arm::B => 'B',
        })
        .collect();
    let mut md = Vec::new();
    md.push(c.render(&Text::new("ab.md.title").arg("tweak", tweak_title)));
    md.push(String::new());
    md.push(
        c.render(
            &Text::new("ab.md.setup")
                .arg("game", game)
                .arg("runs", runs_per_arm)
                .arg("seconds", format!("{duration:.0}"))
                .arg("order", order),
        ),
    );
    md.push(String::new());
    md.push(c.render(&Text::new("ab.md.header")));
    md.push("|---|---|---|---|---|".into());
    md.push(row(c, "ab.md.avg_fps", &report.avg_fps));
    md.push(row(c, "ab.md.low1", &report.low_1pct));
    md.push(String::new());
    let verdict = match report.effect {
        Outcome::Better => "ab.verdict.keep",
        Outcome::Worse => "ab.verdict.worse",
        Outcome::NoEffect => "ab.verdict.no_effect",
    };
    md.push(c.render(&Text::new(verdict)));
    for w in &report.warnings {
        md.push(format!("> ⚠ {}", c.render(w)));
    }
    md.push(String::new());
    md.push(c.render(&Text::new("ab.md.method").arg("min", MIN_EFFECT_PCT)));
    md.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(arm: Arm, avg: f64, low: f64) -> RunResult {
        RunResult {
            arm,
            avg_fps: avg,
            low_1pct_fps: low,
            conditions: RunConditions {
                duration_s: 60.0,
                frames: 8000,
                gpu_temp_avg_c: Some(70.0),
                frame_capped: false,
            },
        }
    }

    fn interleave(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<RunResult> {
        a.iter()
            .zip(b)
            .flat_map(|(x, y)| [run(Arm::A, x.0, x.1), run(Arm::B, y.0, y.1)])
            .collect()
    }

    #[test]
    fn schedule_alternates_with_minimum_three() {
        assert_eq!(
            schedule(1),
            vec![Arm::A, Arm::B, Arm::A, Arm::B, Arm::A, Arm::B]
        );
        assert_eq!(schedule(4).len(), 8);
    }

    #[test]
    fn clear_improvement_is_kept() {
        let r = evaluate(&interleave(
            &[(100.0, 70.0), (101.0, 71.0), (99.0, 69.0)],
            &[(110.0, 80.0), (111.0, 79.0), (109.0, 81.0)],
        ))
        .unwrap();
        assert_eq!(r.avg_fps.outcome, Outcome::Better);
        assert!((r.avg_fps.delta_pct - 10.0).abs() < 1e-9);
        assert!(r.avg_fps.ci_low_pct > 0.0);
        assert_eq!(r.effect, Outcome::Better);
        assert_eq!(r.decision, Decision::Keep);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn noise_is_no_effect_and_reverted() {
        let r = evaluate(&interleave(
            &[(100.0, 70.0), (103.0, 73.0), (97.0, 67.0)],
            &[(101.0, 71.0), (98.0, 68.0), (102.0, 72.0)],
        ))
        .unwrap();
        assert_eq!(r.effect, Outcome::NoEffect);
        assert_eq!(r.decision, Decision::Revert);
        assert!(r.avg_fps.ci_low_pct < 0.0 && r.avg_fps.ci_high_pct > 0.0);
    }

    #[test]
    fn tiny_but_consistent_change_is_not_an_effect() {
        // +0.5 % стабільно — статистично є, практично ні.
        let r = evaluate(&interleave(
            &[(200.0, 150.0), (200.1, 150.0), (199.9, 150.0)],
            &[(201.0, 150.0), (201.1, 150.0), (200.9, 150.0)],
        ))
        .unwrap();
        assert!(r.avg_fps.ci_low_pct > 0.0);
        assert_eq!(r.avg_fps.outcome, Outcome::NoEffect);
        assert_eq!(r.decision, Decision::Revert);
    }

    #[test]
    fn worse_low_1pct_vetoes_better_avg() {
        let r = evaluate(&interleave(
            &[(100.0, 70.0), (101.0, 71.0), (99.0, 69.0)],
            &[(110.0, 60.0), (111.0, 61.0), (109.0, 59.0)],
        ))
        .unwrap();
        assert_eq!(r.avg_fps.outcome, Outcome::Better);
        assert_eq!(r.low_1pct.outcome, Outcome::Worse);
        assert_eq!(r.effect, Outcome::Worse);
        assert_eq!(r.decision, Decision::Revert);
    }

    #[test]
    fn needs_three_runs_each() {
        let runs = interleave(
            &[(100.0, 70.0), (100.0, 70.0)],
            &[(110.0, 80.0), (110.0, 80.0)],
        );
        assert!(matches!(
            evaluate(&runs),
            Err(TweakError::NotEnoughRuns { a: 2, b: 2, .. })
        ));
    }

    #[test]
    fn condition_warnings() {
        let mut runs = interleave(
            &[(100.0, 70.0), (130.0, 71.0), (80.0, 69.0)],
            &[(101.0, 71.0), (102.0, 70.0), (100.0, 70.0)],
        );
        for r in runs.iter_mut().filter(|r| r.arm == Arm::B) {
            r.conditions.gpu_temp_avg_c = Some(80.0);
        }
        runs[0].conditions.frame_capped = true;
        runs[1].conditions.duration_s = 30.0;
        runs[2].conditions.frames = 500;
        let keys: Vec<String> = evaluate(&runs)
            .unwrap()
            .warnings
            .into_iter()
            .map(|t| t.key)
            .collect();
        for k in [
            "ab.warn.temp_drift",
            "ab.warn.duration_mismatch",
            "ab.warn.capped",
            "ab.warn.unstable",
            "ab.warn.few_frames",
        ] {
            assert!(keys.iter().any(|x| x == k), "{k}: {keys:?}");
        }
    }

    #[test]
    fn t_table_is_conservative() {
        assert_eq!(t_critical(4.0), 2.776);
        assert_eq!(t_critical(4.9), 2.776);
        assert_eq!(t_critical(0.5), 12.706);
        assert_eq!(t_critical(f64::INFINITY), 1.96);
    }

    #[test]
    fn markdown_report_is_fully_localized() {
        let c = Catalog::ukrainian().unwrap();
        for (a, b) in [
            (
                [(100.0, 70.0), (101.0, 71.0), (99.0, 69.0)],
                [(110.0, 80.0), (111.0, 79.0), (109.0, 81.0)],
            ),
            (
                [(100.0, 70.0), (103.0, 73.0), (97.0, 67.0)],
                [(101.0, 71.0), (98.0, 68.0), (102.0, 72.0)],
            ),
            (
                [(100.0, 70.0), (101.0, 71.0), (99.0, 69.0)],
                [(90.0, 60.0), (91.0, 61.0), (89.0, 59.0)],
            ),
        ] {
            let mut runs = interleave(&a, &b);
            runs[0].conditions.frame_capped = true;
            let report = evaluate(&runs).unwrap();
            let md = render_markdown(&report, &c, "Схема живлення", "game.exe");
            assert!(!md.contains("ab."), "сирий ключ у звіті:\n{md}");
            assert!(!md.contains('{'), "незаповнений аргумент:\n{md}");
            assert!(md.contains("game.exe"));
            assert!(md.contains("ABABAB"));
        }
    }
}
