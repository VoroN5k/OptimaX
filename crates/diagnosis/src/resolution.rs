//! Тест чутливості FPS до роздільності (CLAUDE.md, розд. 9: GPU-bound — «FPS чутливий до
//! роздільності»). Два прогони на рідній роздільності, потім два на зниженій (роздільність,
//! render scale чи DLSS/FSR Performance змінює користувач у грі) — порядок AABB, щоб у грі
//! перемикати лише раз.
//!
//! Висновок — лише за виміряним: приріст має бути і практично помітним
//! ([`HELPS_MIN_PCT`]), і більшим за розкид між повторами ([`SPREAD_FACTOR`]). Чисті функції.

use crate::{diagnose_timeline, Bottleneck, Context, Diagnosis, MeasuredGain, Timeline, Verdict};
use optimax_core::{Category, Confidence, Finding, FindingBuilder, Impact, Text};
use optimax_frametime::Recording;
use serde::{Deserialize, Serialize};

/// Прогонів на кожну роздільність.
pub const RUNS_PER_ARM: usize = 2;
/// Найменший приріст середнього FPS, з якого кажемо «допоможе», %.
pub const HELPS_MIN_PCT: f64 = 10.0;
/// Зміна FPS, у межах якої кажемо «не допоможе», %.
pub const NO_HELP_MAX_PCT: f64 = 3.0;
/// Приріст має бути в стільки разів більшим за розкид між повторами.
pub const SPREAD_FACTOR: f64 = 2.0;
/// Падіння середнього завантаження GPU (в. п.), яке підтверджує, що роздільність змінилась.
pub const GPU_DROP_MIN_PP: f64 = 5.0;

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

/// Порядок прогонів: `false` — рідна, `true` — знижена.
pub fn schedule() -> [bool; RUNS_PER_ARM * 2] {
    [false, false, true, true]
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
    /// Найбільший розкид середнього FPS між двома повторами однієї роздільності, %.
    pub spread_pct: f64,
    /// Знахідка для показу.
    pub finding: Finding,
    /// Прогони в порядку виконання.
    pub runs: Vec<ResolutionRun>,
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Розкид двох (кількох) значень: (max − min) / середнє, %.
fn spread(v: &[f64]) -> f64 {
    let (Some(m), Some(max), Some(min)) = (
        mean(v),
        v.iter().copied().reduce(f64::max),
        v.iter().copied().reduce(f64::min),
    ) else {
        return 0.0;
    };
    if m > 0.0 {
        (max - min) / m * 100.0
    } else {
        0.0
    }
}

fn change_pct(from: f64, to: f64) -> f64 {
    if from > 0.0 {
        (to - from) / from * 100.0
    } else {
        0.0
    }
}

fn signed_pct(v: f64) -> String {
    format!("{v:+.1}")
}

/// Оцінює прогони. `None` — ще не по [`RUNS_PER_ARM`] прогонів кожної роздільності.
pub fn evaluate(runs: &[ResolutionRun]) -> Option<ResolutionReport> {
    let pick = |reduced: bool, f: fn(&ResolutionRun) -> f64| -> Vec<f64> {
        runs.iter()
            .filter(|r| r.reduced == reduced)
            .map(f)
            .collect()
    };
    let (nat_avg, red_avg) = (pick(false, |r| r.avg_fps), pick(true, |r| r.avg_fps));
    if nat_avg.len() < RUNS_PER_ARM || red_avg.len() < RUNS_PER_ARM {
        return None;
    }
    let (nat_low, red_low) = (
        pick(false, |r| r.low_1pct_fps),
        pick(true, |r| r.low_1pct_fps),
    );
    let (na, ra) = (mean(&nat_avg)?, mean(&red_avg)?);
    let (nl, rl) = (mean(&nat_low)?, mean(&red_low)?);
    let delta = change_pct(na, ra);
    let delta_low = change_pct(nl, rl);
    let spread_pct = spread(&nat_avg).max(spread(&red_avg));

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
    } else if delta >= HELPS_MIN_PCT && delta > SPREAD_FACTOR * spread_pct {
        // Що більший приріст відносно порогу, то впевненіше.
        (
            ResolutionOutcome::Helps,
            (75.0 + (delta - HELPS_MIN_PCT)).min(95.0),
        )
    } else if delta.abs() <= NO_HELP_MAX_PCT {
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
    let mut b = FindingBuilder::new("diag", id, category, impact, confidence.round() as u8)
        .arg("delta", signed_pct(delta))
        .arg("delta_low", signed_pct(delta_low))
        .arg("spread", format!("{spread_pct:.1}"))
        .evidence(
            Text::new("diag.ev.res_avg"),
            Text::new("diag.ev.res_pair")
                .arg("native", format!("{na:.1}"))
                .arg("reduced", format!("{ra:.1}"))
                .arg("delta", signed_pct(delta)),
        )
        .evidence(
            Text::new("diag.ev.res_low"),
            Text::new("diag.ev.res_pair")
                .arg("native", format!("{nl:.1}"))
                .arg("reduced", format!("{rl:.1}"))
                .arg("delta", signed_pct(delta_low)),
        )
        .evidence(
            Text::new("diag.ev.res_spread"),
            Text::new("value.pct").arg("v", format!("{spread_pct:.1}")),
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
        spread_pct,
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
    let gain = MeasuredGain {
        avg_fps_change_pct: report.avg_fps_change_pct,
        low_1pct_change_pct: report.low_1pct_change_pct,
        runs: RUNS_PER_ARM as u32,
    };
    let measured = matches!(
        report.outcome,
        ResolutionOutcome::Helps | ResolutionOutcome::NoHelp
    );
    let test_evidence = || {
        (
            Text::new("diag.ev.res_test"),
            Text::new("diag.ev.res_test_value").arg("delta", signed_pct(report.avg_fps_change_pct)),
        )
    };
    for v in &mut d.verdicts {
        let id = v.finding.id.as_str();
        match (id, report.outcome) {
            ("gpu_bound", ResolutionOutcome::Helps) => {
                let c = v
                    .finding
                    .confidence
                    .percent()
                    .max(report.finding.confidence.percent());
                v.finding.confidence = Confidence::new(c.min(GPU_BOUND_CONFIRMED_MAX));
                v.finding.detail.key = "diag.gpu_bound.detail_confirmed".into();
            }
            ("gpu_bound", ResolutionOutcome::NoHelp) => {
                let c = v
                    .finding
                    .confidence
                    .percent()
                    .min(GPU_BOUND_CONTRADICTED_MAX);
                v.finding.confidence = Confidence::new(c);
                v.finding.detail.key = "diag.gpu_bound.detail_contradicted".into();
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

    fn run(reduced: bool, avg: f64, util: Option<f64>) -> ResolutionRun {
        ResolutionRun {
            reduced,
            avg_fps: avg,
            low_1pct_fps: avg * 0.7,
            gpu_util_avg: util,
            frame_capped: false,
            frames: 5000,
        }
    }

    fn runs(
        native: [f64; 2],
        reduced: [f64; 2],
        util: (Option<f64>, Option<f64>),
    ) -> Vec<ResolutionRun> {
        vec![
            run(false, native[0], util.0),
            run(false, native[1], util.0),
            run(true, reduced[0], util.1),
            run(true, reduced[1], util.1),
        ]
    }

    #[test]
    fn needs_two_runs_of_each() {
        assert!(evaluate(&[]).is_none());
        let mut r = runs([100.0, 100.0], [140.0, 140.0], (Some(99.0), Some(80.0)));
        r.pop();
        assert!(evaluate(&r).is_none());
        assert_eq!(schedule(), [false, false, true, true]);
    }

    #[test]
    fn gpu_bound_game_gains_fps() {
        let r = evaluate(&runs(
            [100.0, 102.0],
            [140.0, 138.0],
            (Some(99.0), Some(85.0)),
        ))
        .unwrap();
        assert_eq!(r.outcome, ResolutionOutcome::Helps);
        assert!((r.avg_fps_change_pct - 37.6).abs() < 0.1);
        assert_eq!(r.finding.id, "res.helps");
        assert_eq!(r.finding.confidence.percent(), 95);
        assert_eq!(r.finding.evidence.len(), 4);
    }

    #[test]
    fn gain_within_spread_is_not_claimed() {
        // +12 %, але повтори розходяться на ~10 % — приріст не вдвічі більший за розкид.
        let r = evaluate(&runs(
            [95.0, 105.0],
            [107.0, 117.0],
            (Some(99.0), Some(85.0)),
        ))
        .unwrap();
        assert_eq!(r.outcome, ResolutionOutcome::Unclear);
    }

    #[test]
    fn cpu_bound_game_does_not_gain() {
        let r = evaluate(&runs(
            [100.0, 101.0],
            [101.0, 102.0],
            (Some(70.0), Some(45.0)),
        ))
        .unwrap();
        assert_eq!(r.outcome, ResolutionOutcome::NoHelp);
        assert_eq!(r.finding.confidence.percent(), 85);
        assert!(r.finding.advice.is_none());
    }

    #[test]
    fn unchanged_gpu_load_means_resolution_was_not_changed() {
        let r = evaluate(&runs(
            [100.0, 101.0],
            [100.0, 101.0],
            (Some(97.0), Some(96.0)),
        ))
        .unwrap();
        assert_eq!(r.outcome, ResolutionOutcome::NotApplied);
        assert!(r.finding.advice.is_some());
    }

    #[test]
    fn without_gpu_metrics_no_help_is_less_certain() {
        let r = evaluate(&runs([100.0, 101.0], [100.0, 101.0], (None, None))).unwrap();
        assert_eq!(r.outcome, ResolutionOutcome::NoHelp);
        assert_eq!(r.finding.confidence.percent(), 60);
        assert_eq!(
            r.finding.advice.as_ref().map(|a| a.key.as_str()),
            Some("diag.res.no_help.advice_no_gpu")
        );
    }

    #[test]
    fn frame_cap_makes_test_meaningless() {
        let mut r = runs([144.0, 144.0], [144.0, 144.0], (Some(60.0), Some(40.0)));
        r[2].frame_capped = true;
        assert_eq!(evaluate(&r).unwrap().outcome, ResolutionOutcome::Capped);
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
        }
    }

    #[test]
    fn confirmed_gpu_bound_gets_measured_gain() {
        let mut d = diagnosis_with(&["summary.gpu", "gpu_bound", "upscale.helps"]);
        let r = evaluate(&runs(
            [100.0, 100.0],
            [140.0, 140.0],
            (Some(99.0), Some(80.0)),
        ))
        .unwrap();
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
        assert!((g.expected_gain.as_ref().unwrap().avg_fps_change_pct - 40.0).abs() < 1e-9);
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
    fn contradicted_gpu_bound_loses_confidence() {
        let mut d = diagnosis_with(&["summary.gpu", "gpu_bound"]);
        let r = evaluate(&runs(
            [100.0, 100.0],
            [101.0, 100.0],
            (Some(99.0), Some(70.0)),
        ))
        .unwrap();
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
        let mut capped = runs([144.0, 144.0], [144.0, 144.0], (Some(60.0), Some(40.0)));
        capped[0].frame_capped = true;
        let cases = [
            runs([100.0, 100.0], [140.0, 140.0], (Some(99.0), Some(80.0))),
            runs([100.0, 100.0], [101.0, 100.0], (Some(99.0), Some(70.0))),
            runs([100.0, 100.0], [101.0, 100.0], (None, None)),
            runs([100.0, 100.0], [100.0, 100.0], (Some(97.0), Some(97.0))),
            runs([100.0, 100.0], [106.0, 106.0], (Some(99.0), Some(90.0))),
            capped,
        ];
        let mut outcomes = Vec::new();
        for c in &cases {
            let r = evaluate(c).unwrap();
            outcomes.push(r.outcome);
            let mut d = diagnosis_with(&["summary.gpu", "gpu_bound", "cpu_bound"]);
            // Заголовки «чужих» вердиктів у моку несправжні — перевіряємо лише змінене тестом.
            apply(&mut d, &r);
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
        let r = evaluate(&runs(
            [100.0, 100.0],
            [106.0, 106.0],
            (Some(99.0), Some(90.0)),
        ))
        .unwrap();
        assert_eq!(r.outcome, ResolutionOutcome::Unclear);
        apply(&mut d, &r);
        assert_eq!(d.verdicts[2], before);
        assert_eq!(d.verdicts[1].finding.id, "res.unclear");
    }
}
