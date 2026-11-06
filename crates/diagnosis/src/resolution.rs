//! Тест чутливості FPS до роздільності (CLAUDE.md, розд. 9: GPU-bound — «FPS чутливий до
//! роздільності»). Прогони на рідній (A) і зниженій (B) роздільності — роздільність, render
//! scale чи DLSS/FSR Performance змінює користувач у грі.
//!
//! Статистика — рушій порівнянь (`optimax_experiment`, правила [`policy`]): порядок ABBA,
//! поправка на дрейф, 3–6 прогонів на роздільність, поки висновок не стане певним.
//! Висновок — лише за виміряним: «допоможе», якщо приріст не менший за [`HELPS_MIN_PCT`], а
//! нижня межа інтервалу вища за [`NO_HELP_MAX_PCT`]; «не допоможе», якщо весь інтервал у
//! межах ±[`NO_HELP_MAX_PCT`].
//! Чисті функції.

use crate::{diagnose_timeline, Bottleneck, Context, Diagnosis, MeasuredGain, Timeline, Verdict};
use optimax_core::{Category, Confidence, Finding, FindingBuilder, Impact, Text};
use optimax_experiment::{Estimate, Outcome, Policy, Report, Status};
use optimax_frametime::Recording;
use serde::{Deserialize, Serialize};

/// Найменший приріст середнього FPS, з якого кажемо «допоможе», %.
pub const HELPS_MIN_PCT: f64 = 10.0;
/// Зміна FPS, у межах якої кажемо «не допоможе», % (поріг «ефекту немає» для рушія).
pub const NO_HELP_MAX_PCT: f64 = 3.0;
/// Падіння середнього завантаження GPU (в. п.), яке підтверджує, що роздільність змінилась.
pub const GPU_DROP_MIN_PP: f64 = 5.0;
/// Найбільше прогонів на кожну роздільність.
pub const MAX_RUNS_PER_ARM: usize = 6;

/// Правила рушія порівнянь для цього тесту: варіант 0 — рідна роздільність, 1 — знижена.
pub fn policy() -> Policy {
    Policy {
        max_per_arm: MAX_RUNS_PER_ARM,
        margin_avg_pct: NO_HELP_MAX_PCT,
        margin_low_pct: 5.0,
        ..Policy::default()
    }
}

/// Один прогін тесту.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResolutionRun {
    /// `false` — рідна роздільність, `true` — знижена.
    pub reduced: bool,
    /// Середній FPS.
    pub avg_fps: f64,
    /// 1% low.
    pub low_1pct_fps: f64,
    /// Середнє завантаження GPU, % (немає — метрик GPU немає).
    pub gpu_util_avg: Option<f64>,
    /// FPS упирався в ліміт кадрів / V-Sync.
    pub frame_capped: bool,
    /// Кадрів у прогоні.
    pub frames: usize,
}

impl ResolutionRun {
    /// Прогін зі шкали часу запису. `None` — даних замало для статистики.
    pub fn from_timeline(tl: &Timeline, reduced: bool) -> Option<Self> {
        let d = diagnose_timeline(tl, &Context::default());
        let stats = d.stats.as_ref()?;
        let util: Vec<f64> = tl
            .samples
            .iter()
            .filter_map(|s| s.gpu.as_ref()?.util.map(f64::from))
            .collect();
        Some(Self {
            reduced,
            avg_fps: stats.avg_fps,
            low_1pct_fps: stats.low_1pct_fps,
            gpu_util_avg: (!util.is_empty()).then(|| util.iter().sum::<f64>() / util.len() as f64),
            frame_capped: d.bottleneck == Bottleneck::FrameCap,
            frames: stats.frames,
        })
    }

    /// Прогін із запису.
    pub fn from_recording(rec: &Recording, reduced: bool) -> Option<Self> {
        Self::from_timeline(&Timeline::from_recording(rec)?, reduced)
    }
}

/// Висновок тесту.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ResolutionOutcome {
    /// Нижча роздільність помітно підняла FPS — обмежує GPU.
    Helps,
    /// FPS майже не змінився, хоча GPU розвантажився — обмежує не GPU.
    NoHelp,
    /// Ні FPS, ні завантаження GPU не змінились — схоже, роздільність не змінили.
    NotApplied,
    /// FPS упирався в ліміт кадрів — тест нічого не показує.
    Capped,
    /// Різниця невелика або в межах розкиду.
    Unclear,
}

/// Підсумок тесту.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResolutionReport {
    /// Висновок.
    pub outcome: ResolutionOutcome,
    /// Зміна середнього FPS, % від рідної роздільності.
    pub avg_fps_change_pct: f64,
    /// Зміна 1% low, %.
    pub low_1pct_change_pct: f64,
    /// Інтервал зміни середнього FPS: нижня межа, %.
    pub ci_low_pct: f64,
    /// Верхня межа, %.
    pub ci_high_pct: f64,
    /// Прогонів на кожну роздільність.
    pub runs_per_arm: u32,
    /// Знахідка для показу.
    pub finding: Finding,
    /// Прогони в порядку виконання.
    pub runs: Vec<ResolutionRun>,
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn signed_pct(v: f64) -> String {
    format!("{v:+.1}")
}

/// Підсумок за оцінкою рушія. `None` — висновку ще немає (рушій просить ще прогони).
/// `runs` — ті самі прогони в тому самому порядку, що й у рушія.
pub fn evaluate(runs: &[ResolutionRun], estimate: &Report) -> Option<ResolutionReport> {
    if estimate.status != Status::Done {
        return None;
    }
    let cmp = estimate.comparison(1)?;
    let (avg, low): (&Estimate, &Estimate) = (cmp.avg_fps.as_ref()?, cmp.low_1pct.as_ref()?);
    let delta = avg.delta_pct;
    let delta_low = low.delta_pct;
    let per_arm = estimate.runs_of(0).min(estimate.runs_of(1));

    let util = |reduced: bool| -> Option<f64> {
        let v: Vec<f64> = runs
            .iter()
            .filter(|r| r.reduced == reduced)
            .map(|r| r.gpu_util_avg)
            .collect::<Option<Vec<f64>>>()?;
        mean(&v)
    };
    let (nat_util, red_util) = (util(false), util(true));
    let gpu_drop = nat_util.zip(red_util).map(|(n, r)| n - r);

    let capped = runs.iter().any(|r| r.frame_capped);
    let (outcome, confidence) = if capped {
        (ResolutionOutcome::Capped, 90.0)
    } else if avg.outcome == Outcome::Better
        && delta >= HELPS_MIN_PCT
        && avg.ci_low_pct >= NO_HELP_MAX_PCT
    {
        // Приріст значущий і напевно більший за смугу «не допоможе»; що більший він
        // відносно порогу, то впевненіше.
        (
            ResolutionOutcome::Helps,
            (75.0 + (delta - HELPS_MIN_PCT)).min(95.0),
        )
    } else if avg.outcome == Outcome::NoEffect {
        match gpu_drop {
            Some(d) if d < GPU_DROP_MIN_PP => (ResolutionOutcome::NotApplied, 70.0),
            Some(_) => (ResolutionOutcome::NoHelp, 85.0),
            // Без метрик GPU не перевірити, чи роздільність справді змінилась.
            None => (ResolutionOutcome::NoHelp, 60.0),
        }
    } else {
        (ResolutionOutcome::Unclear, 50.0)
    };

    let (id, impact) = match outcome {
        ResolutionOutcome::Helps => ("res.helps", Impact::Moderate),
        ResolutionOutcome::NoHelp => ("res.no_help", Impact::Info),
        ResolutionOutcome::NotApplied => ("res.not_applied", Impact::Info),
        ResolutionOutcome::Capped => ("res.capped", Impact::Info),
        ResolutionOutcome::Unclear => ("res.unclear", Impact::Info),
    };
    let category = match outcome {
        ResolutionOutcome::Helps => Category::Gpu,
        ResolutionOutcome::Capped => Category::FrameCap,
        _ => Category::Summary,
    };
    let (lo, hi) = (signed_pct(avg.ci_low_pct), signed_pct(avg.ci_high_pct));
    let mut b = FindingBuilder::new("diag", id, category, impact, confidence.round() as u8)
        .arg("delta", signed_pct(delta))
        .arg("delta_low", signed_pct(delta_low))
        .arg("lo", lo.clone())
        .arg("hi", hi.clone())
        .evidence(
            Text::new("diag.ev.res_avg"),
            Text::new("diag.ev.res_pair")
                .arg("native", format!("{:.1}", avg.a))
                .arg("reduced", format!("{:.1}", avg.b))
                .arg("delta", signed_pct(delta)),
        )
        .evidence(
            Text::new("diag.ev.res_low"),
            Text::new("diag.ev.res_pair")
                .arg("native", format!("{:.1}", low.a))
                .arg("reduced", format!("{:.1}", low.b))
                .arg("delta", signed_pct(delta_low)),
        )
        .evidence(
            Text::new("diag.ev.res_ci"),
            Text::new("diag.ev.res_ci_value")
                .arg("lo", lo)
                .arg("hi", hi)
                .arg("conf", format!("{:.1}", estimate.confidence_pct))
                .arg("n", per_arm),
        );
    if let (Some(n), Some(r)) = (nat_util, red_util) {
        b = b.evidence(
            Text::new("diag.ev.res_gpu"),
            Text::new("diag.ev.res_gpu_value")
                .arg("native", format!("{n:.0}"))
                .arg("reduced", format!("{r:.0}")),
        );
    }
    if outcome != ResolutionOutcome::NoHelp || gpu_drop.is_none() {
        b = b.advice();
    }
    // Для «не допоможе» без метрик GPU — окрема порада (перевірити, що роздільність змінилась).
    let finding = if outcome == ResolutionOutcome::NoHelp && gpu_drop.is_none() {
        let mut f = b.build();
        if let Some(a) = f.advice.as_mut() {
            a.key = "diag.res.no_help.advice_no_gpu".into();
        }
        f
    } else {
        b.build()
    };
    Some(ResolutionReport {
        outcome,
        avg_fps_change_pct: delta,
        low_1pct_change_pct: delta_low,
        ci_low_pct: avg.ci_low_pct,
        ci_high_pct: avg.ci_high_pct,
        runs_per_arm: u32::try_from(per_arm).unwrap_or(u32::MAX),
        finding,
        runs: runs.to_vec(),
    })
}

/// Найбільша впевненість GPU-bound, коли тест її не підтвердив.
const GPU_BOUND_CONTRADICTED_MAX: u8 = 40;
/// Найбільша впевненість GPU-bound, підтвердженого тестом.
const GPU_BOUND_CONFIRMED_MAX: u8 = 95;

/// Переносить результат тесту в діагноз: вердикт тесту одразу після підсумку (замість
/// попереднього), уточнена впевненість GPU-bound і виміряний ефект там, де його обіцяють.
pub fn apply(d: &mut Diagnosis, report: &ResolutionReport) {
    d.verdicts.retain(|v| !v.finding.id.starts_with("res."));
    // Підказка про Reflex спирається на те, що обмежує відеокарта, а тест цього не підтвердив.
    if report.outcome == ResolutionOutcome::NoHelp {
        d.verdicts.retain(|v| v.finding.id != "latency_queue");
    }
    let gain = MeasuredGain {
        avg_fps_change_pct: report.avg_fps_change_pct,
        low_1pct_change_pct: report.low_1pct_change_pct,
        runs: report.runs_per_arm,
    };
    let measured = matches!(
        report.outcome,
        ResolutionOutcome::Helps | ResolutionOutcome::NoHelp
    );
    let test_evidence = || {
        (
            Text::new("diag.ev.res_test"),
            Text::new("diag.ev.res_test_value")
                .arg("delta", signed_pct(report.avg_fps_change_pct))
                .arg("n", report.runs_per_arm),
        )
    };
    for v in &mut d.verdicts {
        let id = v.finding.id.as_str();
        // Межовий вердикт (GPU не в стелі, але зайнятий майже весь кадр) має власні тексти.
        let edge = v.finding.detail.key.ends_with(".detail_edge");
        match (id, report.outcome) {
            ("gpu_bound", ResolutionOutcome::Helps) => {
                let c = v
                    .finding
                    .confidence
                    .percent()
                    .max(report.finding.confidence.percent());
                v.finding.confidence = Confidence::new(c.min(GPU_BOUND_CONFIRMED_MAX));
                v.finding.detail.key = if edge {
                    "diag.gpu_bound.detail_edge_confirmed"
                } else {
                    "diag.gpu_bound.detail_confirmed"
                }
                .into();
            }
            ("gpu_bound", ResolutionOutcome::NoHelp) => {
                let c = v
                    .finding
                    .confidence
                    .percent()
                    .min(GPU_BOUND_CONTRADICTED_MAX);
                v.finding.confidence = Confidence::new(c);
                v.finding.detail.key = if edge {
                    "diag.gpu_bound.detail_edge_contradicted"
                } else {
                    "diag.gpu_bound.detail_contradicted"
                }
                .into();
            }
            _ => {}
        }
        let relevant = matches!(
            id,
            "gpu_bound" | "cpu_bound" | "upscale.helps" | "upscale.no_help"
        );
        if relevant && measured {
            let (label, value) = test_evidence();
            v.finding
                .evidence
                .retain(|e| e.label.key != "diag.ev.res_test");
            v.finding
                .evidence
                .push(optimax_core::Evidence::new(label, value));
            v.expected_gain = Some(gain.clone());
        }
    }
    // Підсумок «вузьке місце — відеокарта» має ту саму впевненість, що й уточнений GPU-bound.
    let gpu_confidence = d
        .verdicts
        .iter()
        .find(|v| v.finding.id == "gpu_bound")
        .map(|v| v.finding.confidence);
    if let (Some(c), Some(first)) = (gpu_confidence, d.verdicts.first_mut()) {
        if first.finding.id == "summary.gpu" {
            first.finding.confidence = c;
        }
    }
    let at = d.verdicts.len().min(1);
    d.verdicts.insert(at, Verdict::from(report.finding.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    use optimax_experiment::{next_variant, Run};

    /// Прогін із `avg` FPS: 3000 кадрів із детермінованим шумом ±5 % (середнє рівно `avg`).
    fn exp_run(reduced: bool, avg: f64, seed: usize) -> Run {
        let base = 1000.0 / avg;
        let mut ft: Vec<f64> = (0..3000)
            .map(|i| {
                let u = ((i * 7919 + seed * 104_729) % 101) as f64 / 50.0 - 1.0;
                base * (1.0 + 0.05 * u)
            })
            .collect();
        let m = ft.iter().sum::<f64>() / ft.len() as f64;
        for v in &mut ft {
            *v *= base / m;
        }
        Run {
            variant: u8::from(reduced),
            frametimes_ms: ft,
            gpu_temp_avg_c: Some(70.0),
            frame_capped: false,
            excluded: false,
        }
    }

    /// Тест до висновку рушія (або до `limit` прогонів): FPS рідної / зниженої з розкидом
    /// між прогонами ±`jitter` (частка), завантаження GPU — `util`.
    fn test_runs(
        native: f64,
        reduced: f64,
        jitter: f64,
        util: (Option<f64>, Option<f64>),
        limit: usize,
    ) -> (Vec<ResolutionRun>, Report) {
        let mut exp: Vec<Run> = Vec::new();
        let mut res = Vec::new();
        loop {
            let report = optimax_experiment::evaluate(&exp, &policy());
            if report.status == Status::Done || exp.len() >= limit {
                return (res, report);
            }
            let r = next_variant(&exp, 2) == 1;
            let i = exp.len();
            let k = 1.0 + jitter * if i.is_multiple_of(3) { 1.0 } else { -0.5 };
            let fps = if r { reduced } else { native } * k;
            exp.push(exp_run(r, fps, i));
            res.push(ResolutionRun {
                reduced: r,
                avg_fps: fps,
                low_1pct_fps: fps * 0.7,
                gpu_util_avg: if r { util.1 } else { util.0 },
                frame_capped: false,
                frames: 3000,
            });
        }
    }

    fn report_for(native: f64, reduced: f64, util: (Option<f64>, Option<f64>)) -> ResolutionReport {
        let (runs, est) = test_runs(native, reduced, 0.003, util, 100);
        evaluate(&runs, &est).expect("висновок")
    }

    #[test]
    fn needs_a_certain_estimate() {
        let (runs, est) = test_runs(100.0, 140.0, 0.003, (Some(99.0), Some(80.0)), 4);
        assert_eq!(est.status, Status::Continue);
        assert!(evaluate(&runs, &est).is_none());
        let (runs, est) = test_runs(100.0, 140.0, 0.003, (Some(99.0), Some(80.0)), 100);
        assert_eq!(runs.len(), 6, "ABBAAB вистачає для явного приросту");
        let order: Vec<bool> = runs.iter().map(|r| r.reduced).collect();
        assert_eq!(order, [false, true, true, false, false, true]);
        assert!(evaluate(&runs, &est).is_some());
    }

    #[test]
    fn gpu_bound_game_gains_fps() {
        let r = report_for(100.0, 138.0, (Some(99.0), Some(85.0)));
        assert_eq!(r.outcome, ResolutionOutcome::Helps);
        assert!((r.avg_fps_change_pct - 38.0).abs() < 1.0, "{r:?}");
        assert!(r.ci_low_pct > HELPS_MIN_PCT);
        assert_eq!(r.finding.id, "res.helps");
        assert_eq!(r.finding.confidence.percent(), 95);
        assert_eq!(r.finding.evidence.len(), 4);
        assert_eq!(r.runs_per_arm, 3);
    }

    #[test]
    fn small_gain_is_not_claimed() {
        // +6 % — значущо, але менше за поріг «допоможе».
        let r = report_for(100.0, 106.0, (Some(99.0), Some(90.0)));
        assert_eq!(r.outcome, ResolutionOutcome::Unclear);
    }

    #[test]
    fn noisy_runs_ask_for_more_and_stay_unclear() {
        // Розкид між прогонами ±8 % — навіть +12 % не можна стверджувати певно.
        let (runs, est) = test_runs(100.0, 112.0, 0.08, (Some(99.0), Some(85.0)), 100);
        assert!(runs.len() > 6, "{}", runs.len());
        let r = evaluate(&runs, &est).expect("висновок на межі прогонів");
        assert_ne!(r.outcome, ResolutionOutcome::Helps, "{r:?}");
    }

    #[test]
    fn cpu_bound_game_does_not_gain() {
        let r = report_for(100.0, 100.5, (Some(70.0), Some(45.0)));
        assert_eq!(r.outcome, ResolutionOutcome::NoHelp);
        assert_eq!(r.finding.confidence.percent(), 85);
        assert!(r.finding.advice.is_none());
    }

    #[test]
    fn unchanged_gpu_load_means_resolution_was_not_changed() {
        let r = report_for(100.0, 100.0, (Some(97.0), Some(96.0)));
        assert_eq!(r.outcome, ResolutionOutcome::NotApplied);
        assert!(r.finding.advice.is_some());
    }

    #[test]
    fn without_gpu_metrics_no_help_is_less_certain() {
        let r = report_for(100.0, 100.5, (None, None));
        assert_eq!(r.outcome, ResolutionOutcome::NoHelp);
        assert_eq!(r.finding.confidence.percent(), 60);
        assert_eq!(
            r.finding.advice.as_ref().map(|a| a.key.as_str()),
            Some("diag.res.no_help.advice_no_gpu")
        );
    }

    #[test]
    fn frame_cap_makes_test_meaningless() {
        let (mut runs, est) = test_runs(144.0, 144.0, 0.0, (Some(60.0), Some(40.0)), 100);
        runs[2].frame_capped = true;
        assert_eq!(
            evaluate(&runs, &est).expect("висновок").outcome,
            ResolutionOutcome::Capped
        );
    }

    fn diagnosis_with(ids: &[&str]) -> Diagnosis {
        let verdicts = ids
            .iter()
            .map(|id| {
                let mut f =
                    FindingBuilder::new("diag", "x", Category::Gpu, Impact::Info, 80).build();
                f.id = (*id).into();
                f.detail = Text::new(format!("diag.{id}.detail")).arg("share", "90");
                Verdict::from(f)
            })
            .collect();
        Diagnosis {
            game: "game.exe".into(),
            stats: None,
            bottleneck: Bottleneck::Gpu,
            verdicts,
            chart: None,
            frame_gen: None,
            anatomy: None,
            latency: None,
            view: None,
        }
    }

    #[test]
    fn confirmed_gpu_bound_gets_measured_gain() {
        let mut d = diagnosis_with(&["summary.gpu", "gpu_bound", "upscale.helps"]);
        let r = report_for(100.0, 140.0, (Some(99.0), Some(80.0)));
        apply(&mut d, &r);
        let ids: Vec<_> = d.verdicts.iter().map(|v| v.finding.id.as_str()).collect();
        assert_eq!(
            ids,
            ["summary.gpu", "res.helps", "gpu_bound", "upscale.helps"]
        );
        let g = &d.verdicts[2];
        assert_eq!(g.finding.confidence.percent(), 95);
        assert_eq!(d.verdicts[0].finding.confidence.percent(), 95);
        assert_eq!(g.finding.detail.key, "diag.gpu_bound.detail_confirmed");
        assert_eq!(g.finding.detail.args["share"], "90");
        let gain = g.expected_gain.as_ref().unwrap();
        assert!((gain.avg_fps_change_pct - 40.0).abs() < 1.0);
        assert_eq!(gain.runs, 3);
        assert!(d.verdicts[3].expected_gain.is_some());
        // Повторний тест замінює попередній вердикт і доказ, а не дублює.
        apply(&mut d, &r);
        assert_eq!(d.verdicts.len(), 4);
        let n = d.verdicts[2]
            .finding
            .evidence
            .iter()
            .filter(|e| e.label.key == "diag.ev.res_test")
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn edge_gpu_bound_keeps_its_own_texts_after_the_test() {
        let edge = |d: &mut Diagnosis| {
            for v in &mut d.verdicts {
                if v.finding.id == "gpu_bound" {
                    v.finding.detail.key = "diag.gpu_bound.detail_edge".into();
                }
            }
        };
        let key = |d: &Diagnosis| {
            d.verdicts
                .iter()
                .find(|v| v.finding.id == "gpu_bound")
                .map(|v| v.finding.detail.key.clone())
                .unwrap()
        };
        let helps = report_for(100.0, 140.0, (Some(99.0), Some(80.0)));
        let mut d = diagnosis_with(&["summary.gpu", "gpu_bound"]);
        edge(&mut d);
        apply(&mut d, &helps);
        assert_eq!(key(&d), "diag.gpu_bound.detail_edge_confirmed");

        let no = report_for(100.0, 100.5, (Some(99.0), Some(70.0)));
        let mut d = diagnosis_with(&["summary.gpu", "gpu_bound"]);
        edge(&mut d);
        apply(&mut d, &no);
        assert_eq!(key(&d), "diag.gpu_bound.detail_edge_contradicted");
    }

    #[test]
    fn contradicted_gpu_bound_drops_reflex_hint() {
        let hint = |outcome_fps: f64| {
            let (runs, est) = test_runs(60.0, outcome_fps, 0.0, (Some(60.0), Some(40.0)), 100);
            let report = evaluate(&runs, &est).expect("висновок");
            let mut d = diagnosis_with(&["summary.gpu", "gpu_bound", "latency_queue"]);
            apply(&mut d, &report);
            (
                report.outcome,
                d.verdicts.iter().any(|v| v.finding.id == "latency_queue"),
            )
        };
        assert_eq!(hint(60.0), (ResolutionOutcome::NoHelp, false));
        assert_eq!(hint(90.0), (ResolutionOutcome::Helps, true));
    }

    #[test]
    fn contradicted_gpu_bound_loses_confidence() {
        let mut d = diagnosis_with(&["summary.gpu", "gpu_bound"]);
        let r = report_for(100.0, 100.5, (Some(99.0), Some(70.0)));
        apply(&mut d, &r);
        let g = d
            .verdicts
            .iter()
            .find(|v| v.finding.id == "gpu_bound")
            .unwrap();
        assert_eq!(g.finding.confidence.percent(), 40);
        assert_eq!(g.finding.detail.key, "diag.gpu_bound.detail_contradicted");
        // Заголовок діагнозу показує ту саму впевненість.
        assert_eq!(d.verdicts[0].finding.confidence.percent(), 40);
    }

    #[test]
    fn every_text_is_localized() {
        let catalog = optimax_core::Catalog::ukrainian().unwrap();
        let (mut capped, capped_est) =
            test_runs(144.0, 144.0, 0.003, (Some(60.0), Some(40.0)), 100);
        capped[0].frame_capped = true;
        let cases = [
            report_for(100.0, 140.0, (Some(99.0), Some(80.0))),
            report_for(100.0, 100.5, (Some(99.0), Some(70.0))),
            report_for(100.0, 100.5, (None, None)),
            report_for(100.0, 100.0, (Some(97.0), Some(97.0))),
            report_for(100.0, 106.0, (Some(99.0), Some(90.0))),
            evaluate(&capped, &capped_est).unwrap(),
        ];
        let mut outcomes = Vec::new();
        for r in &cases {
            outcomes.push(r.outcome);
            let mut d = diagnosis_with(&["summary.gpu", "gpu_bound", "cpu_bound"]);
            // Заголовки «чужих» вердиктів у моку несправжні — перевіряємо лише змінене тестом.
            apply(&mut d, r);
            let mut texts = vec![];
            for v in &d.verdicts {
                let f = &v.finding;
                if f.id.starts_with("res.") {
                    texts.extend([f.title.clone(), f.detail.clone()]);
                    texts.extend(f.advice.clone());
                }
                if f.id == "gpu_bound" {
                    texts.push(f.detail.clone());
                }
                for e in &f.evidence {
                    texts.extend([e.label.clone(), e.value.clone()]);
                }
            }
            for t in texts {
                if t.key == "diag.gpu_bound.detail" || t.key == "diag.cpu_bound.detail" {
                    continue;
                }
                assert!(catalog.contains(&t.key), "немає ключа в uk.json: {}", t.key);
                let s = catalog.render(&t);
                assert!(!s.contains('{'), "незаповнений аргумент у {}: {s}", t.key);
            }
        }
        use ResolutionOutcome::*;
        assert_eq!(
            outcomes,
            [Helps, NoHelp, NoHelp, NotApplied, Unclear, Capped]
        );
    }

    #[test]
    fn unclear_result_changes_nothing_but_adds_its_verdict() {
        let mut d = diagnosis_with(&["summary.gpu", "gpu_bound"]);
        let before = d.verdicts[1].clone();
        let r = report_for(100.0, 106.0, (Some(99.0), Some(90.0)));
        assert_eq!(r.outcome, ResolutionOutcome::Unclear);
        apply(&mut d, &r);
        assert_eq!(d.verdicts[2], before);
        assert_eq!(d.verdicts[1].finding.id, "res.unclear");
    }
}
