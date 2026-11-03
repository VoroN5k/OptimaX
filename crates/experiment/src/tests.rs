use super::*;
use crate::stats::{normal_quantile, SplitMix};
use crate::thermal::thermal_state;

/// Прогін: `frames` кадрів із середнім FPS `fps`, шум кадру ±8 %, кожен 150-й кадр — ривок ×`spike`.
fn run_with(arm: Arm, fps: f64, frames: usize, spike: f64, seed: u64) -> Run {
    let mut rng = SplitMix::new(seed);
    let base = 1000.0 / fps;
    let mut ft: Vec<f64> = (0..frames)
        .map(|i| {
            let u = rng.below(10_001) as f64 / 10_000.0 * 2.0 - 1.0;
            let k = if i % 150 == 75 { spike } else { 1.0 };
            base * (1.0 + 0.08 * u) * k
        })
        .collect();
    // Підганяємо середнє рівно під `fps`, щоб рівень задавав тест, а не шум.
    let mean = ft.iter().sum::<f64>() / ft.len() as f64;
    for v in &mut ft {
        *v *= base / mean;
    }
    Run {
        arm,
        frametimes_ms: ft,
        gpu_temp_avg_c: Some(70.0),
        frame_capped: false,
        excluded: false,
    }
}

fn run(arm: Arm, fps: f64, seed: u64) -> Run {
    run_with(arm, fps, 4000, 2.5, seed)
}

/// Прогони за порядком ABBA, рівень FPS прогону — `level(номер, варіант)`.
fn series(n: usize, level: impl Fn(usize, Arm) -> f64) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for i in 0..n {
        let arm = next_arm(&runs);
        runs.push(run(arm, level(i, arm), 1000 + i as u64));
    }
    runs
}

/// Детермінований шум прогону ±`amp` (частка).
fn noise(i: usize, amp: f64) -> f64 {
    let mut r = SplitMix::new(77 + i as u64);
    (r.below(10_001) as f64 / 10_000.0 * 2.0 - 1.0) * amp
}

#[test]
fn quantiles_match_reference_tables() {
    assert!((normal_quantile(0.975) - 1.959_964).abs() < 1e-5);
    assert!((normal_quantile(0.005) + 2.575_829).abs() < 1e-5);
    for (df, t) in [
        (1.0, 12.706),
        (2.0, 4.303),
        (3.0, 3.182),
        (5.0, 2.571),
        (10.0, 2.228),
        (30.0, 2.042),
    ] {
        let got = stats::t_quantile(0.975, df);
        assert!((got - t).abs() / t < 0.005, "df {df}: {got} проти {t}");
    }
    // 99,5 % при df = 4 — 4.604.
    assert!((stats::t_quantile(0.995, 4.0) - 4.604).abs() / 4.604 < 0.01);
}

#[test]
fn order_is_abba_and_repeats_rebalance() {
    let mut runs: Vec<Run> = Vec::new();
    let mut order = String::new();
    for _ in 0..8 {
        let arm = next_arm(&runs);
        order.push(if arm == Arm::A { 'A' } else { 'B' });
        runs.push(run(arm, 100.0, 1));
    }
    assert_eq!(order, "ABBAABBA");
    // Виключений прогін B — наступним знову B.
    runs[5].excluded = true;
    assert_eq!(next_arm(&runs), Arm::B);
}

#[test]
fn low1_matches_frame_stats() {
    let r = run(Arm::A, 120.0, 9);
    let s = optimax_frametime::frame_stats(&r.frametimes_ms).expect("кадри є");
    let got = low1_fps(&mut r.frametimes_ms.clone());
    assert!((got - s.low_1pct_fps).abs() < 1e-9);
}

#[test]
fn linear_drift_is_not_mistaken_for_an_effect() {
    // FPS падає на 2 за прогін (прогрів), зміна нічого не дає.
    let runs = series(8, |i, _| 100.0 - 2.0 * i as f64);
    // ABBA: на повних четвірках лінійний дрейф ділиться порівну; модель його ще й оцінює.
    let rep = evaluate(&runs, &Policy::default());
    let avg = rep.avg_fps.expect("оцінка");
    assert!(avg.delta_pct.abs() < 0.3, "{avg:?}");
    assert!((avg.drift_pct_per_run + 2.2).abs() < 0.3, "{avg:?}");
    assert_eq!(avg.outcome, Outcome::NoEffect);
    assert_eq!(rep.status, Status::Done);
    assert_eq!(rep.verdict, Outcome::NoEffect);
    assert!(rep.warnings.iter().any(|w| w.key == "exp.warn.drift"));
}

#[test]
fn clear_gain_stops_after_minimum_runs() {
    let runs = series(6, |i, arm| {
        let base = 100.0 * (1.0 + noise(i, 0.01));
        if arm == Arm::B {
            base * 1.06
        } else {
            base
        }
    });
    let rep = evaluate(&runs, &Policy::default());
    assert_eq!(rep.status, Status::Done, "{rep:?}");
    assert_eq!(rep.verdict, Outcome::Better);
    let avg = rep.avg_fps.expect("оцінка");
    assert!((avg.delta_pct - 6.0).abs() < 2.0, "{avg:?}");
    assert!(avg.ci_low_pct > 0.0);
    assert!(rep.confidence_pct > 98.0);
}

#[test]
fn clear_loss_is_reported_as_worse() {
    let runs = series(6, |i, arm| {
        let base = 100.0 * (1.0 + noise(i, 0.01));
        if arm == Arm::B {
            base * 0.93
        } else {
            base
        }
    });
    let rep = evaluate(&runs, &Policy::default());
    assert_eq!(rep.verdict, Outcome::Worse);
}

#[test]
fn unbalanced_or_too_few_runs_continue_with_next_arm() {
    let runs = series(5, |_, _| 100.0);
    let rep = evaluate(&runs, &Policy::default());
    assert_eq!(rep.status, Status::Continue);
    assert_eq!(rep.next, Some(Arm::B));
    assert_eq!(rep.verdict, Outcome::Unclear);
    let rep = evaluate(&runs[..4], &Policy::default());
    assert!(rep.avg_fps.is_none());
    assert_eq!(rep.next, Some(Arm::A));
}

#[test]
fn noisy_small_effect_asks_for_more_and_ends_unclear_at_max() {
    let level = |i: usize, arm: Arm| {
        let base = 100.0 * (1.0 + noise(i, 0.06));
        if arm == Arm::B {
            base * 1.015
        } else {
            base
        }
    };
    let rep = evaluate(&series(6, level), &Policy::default());
    assert_eq!(rep.status, Status::Continue, "{rep:?}");
    assert!(rep.more_pairs.is_some_and(|n| n >= 1));
    let rep = evaluate(&series(16, level), &Policy::default());
    assert_eq!(rep.status, Status::Done);
    assert_eq!(rep.verdict, Outcome::Unclear, "{:?}", rep.avg_fps);
    assert!(rep.next.is_none());
}

#[test]
fn different_scene_is_flagged_and_exclusion_is_honoured() {
    let mut runs = series(8, |i, _| 100.0 * (1.0 + noise(i, 0.005)));
    // Прогін 4 (A): інша сцена — FPS на 12 % нижчий.
    runs[4] = run(Arm::A, 88.0, 4242);
    // Прогін 5 (B): той самий рівень, але вдвічі більше довгих кадрів.
    runs[5] = run_with(Arm::B, 100.0, 4000, 6.0, 5151);
    let rep = evaluate(&runs, &Policy::default());
    assert_eq!(rep.runs[4].suspect, Some(Suspect::Level));
    assert_eq!(rep.runs[5].suspect, Some(Suspect::Shape));
    assert!(rep
        .runs
        .iter()
        .enumerate()
        .all(|(i, r)| i == 4 || i == 5 || r.suspect.is_none()));
    assert!(rep
        .warnings
        .iter()
        .any(|w| w.key == "exp.warn.suspect_level"));
    // Користувач виключив обидва.
    runs[4].excluded = true;
    runs[5].excluded = true;
    let rep = evaluate(&runs, &Policy::default());
    assert!(rep.runs[4].excluded && rep.runs[4].suspect.is_none());
    // По 3 на варіант без підозрілих: різниця мала, але інтервал ще ширший за ±1 % —
    // тест просить наступну пару (четверта пара — BA).
    let avg = rep.avg_fps.expect("оцінка без виключених");
    assert!(avg.delta_pct.abs() < 1.0, "{avg:?}");
    assert_eq!(rep.status, Status::Continue);
    assert_eq!(rep.next, Some(Arm::B));
}

#[test]
fn low_precision_is_deterministic_and_tighter_for_longer_runs() {
    let short = run_with(Arm::A, 100.0, 1500, 2.5, 3);
    let long = run_with(Arm::A, 100.0, 12_000, 2.5, 3);
    let p1 = low_precision_pct(&short.frametimes_ms, 1).expect("досить кадрів");
    assert_eq!(Some(p1), low_precision_pct(&short.frametimes_ms, 1));
    let p2 = low_precision_pct(&long.frametimes_ms, 1).expect("досить кадрів");
    assert!(p2 < p1, "{p2} < {p1}");
    assert!(low_precision_pct(&short.frametimes_ms[..100], 1).is_none());
}

#[test]
fn warnings_cover_conditions() {
    let mut runs = series(6, |_, _| 100.0);
    for r in runs.iter_mut().filter(|r| r.arm == Arm::B) {
        r.gpu_temp_avg_c = Some(78.0);
    }
    runs[0].frame_capped = true;
    runs[1].frametimes_ms.truncate(500);
    let rep = evaluate(&runs, &Policy::default());
    let keys: Vec<&str> = rep.warnings.iter().map(|w| w.key.as_str()).collect();
    for k in [
        "exp.warn.temp_drift",
        "exp.warn.capped",
        "exp.warn.few_frames",
        "exp.warn.duration_mismatch",
    ] {
        assert!(keys.contains(&k), "{k} у {keys:?}");
    }
}

#[test]
fn thermal_plateau_needs_a_full_flat_window() {
    let rising: Vec<(f64, f64)> = (0..120)
        .map(|s| (f64::from(s), 50.0 + f64::from(s) / 30.0))
        .collect();
    let t = thermal_state(&rising);
    assert!(!t.stable);
    assert!((t.slope_c_per_min.expect("нахил") - 2.0).abs() < 0.05);
    let flat: Vec<(f64, f64)> = (0..90)
        .map(|s| (f64::from(s), 71.0 + if s % 2 == 0 { 0.2 } else { -0.2 }))
        .collect();
    assert!(thermal_state(&flat).stable);
    assert!(!thermal_state(&flat[..30]).stable);
    assert!(!thermal_state(&[]).stable);
}

#[test]
fn every_warning_has_a_ukrainian_text() {
    let catalog = optimax_core::Catalog::ukrainian().expect("каталог");
    let src = include_str!("lib.rs");
    let keys: Vec<&str> = src
        .match_indices("\"exp.warn.")
        .filter_map(|(i, _)| src[i + 1..].split('"').next())
        .collect();
    assert!(keys.len() >= 8, "{keys:?}");
    for k in keys {
        assert!(catalog.contains(k), "немає тексту {k}");
        let rendered = catalog.render(
            &Text::new(k)
                .arg("a", 1)
                .arg("b", 2)
                .arg("pct", 3)
                .arg("min", 4)
                .arg("max", 5)
                .arg("n", 6)
                .arg("arm", "A"),
        );
        assert!(!rendered.contains('{'), "{k}: {rendered}");
    }
}
