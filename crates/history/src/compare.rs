//! Порівняння записів: зміна FPS проти похибки й що змінилося в стані ПК.

use crate::model::{scene_match, HistoryEntry, SceneMatch, StateChange, StateField, SystemState};
use serde::Serialize;

/// Менша зміна FPS не вважається значущою навіть за малого розкиду: різні моменти гри в одній
/// сцені, температура, фонові задачі дають кілька відсотків і без жодних змін у системі.
pub const MIN_SIGNIFICANT_PCT: f64 = 8.0;
/// Посекундних відрізків менше за це — розкид не оцінюємо.
const MIN_SECONDS: usize = 10;

/// Розкид середнього FPS усередині запису, % від середнього: стандартна похибка середнього за
/// посекундним FPS (`σ / √n / середнє`). Відрізки — за часом кадрів, неповна остання секунда
/// відкидається. `None` — замало секунд.
pub fn fps_noise_pct(frametimes_ms: &[f64]) -> Option<f64> {
    let mut per_second = Vec::new();
    let (mut acc_ms, mut frames) = (0.0, 0u32);
    for &ft in frametimes_ms.iter().filter(|f| f.is_finite() && **f > 0.0) {
        acc_ms += ft;
        frames += 1;
        if acc_ms >= 1000.0 {
            per_second.push(f64::from(frames) * 1000.0 / acc_ms);
            acc_ms = 0.0;
            frames = 0;
        }
    }
    if per_second.len() < MIN_SECONDS {
        return None;
    }
    let n = per_second.len() as f64;
    let mean = per_second.iter().sum::<f64>() / n;
    if mean <= 0.0 {
        return None;
    }
    let var = per_second.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(var.sqrt() / n.sqrt() / mean * 100.0)
}

/// Що змінилося в стані ПК від `a` до `b`. Невідоме з будь-якого боку зміною не вважається.
pub fn diff(a: &SystemState, b: &SystemState) -> Vec<StateChange> {
    let mut out = Vec::new();
    let mut push = |field: StateField, from: Option<String>, to: Option<String>| {
        if let (Some(from), Some(to)) = (from, to) {
            if from != to {
                out.push(StateChange { field, from, to });
            }
        }
    };
    push(StateField::Gpu, a.gpu.clone(), b.gpu.clone());
    push(StateField::Driver, a.driver.clone(), b.driver.clone());
    push(StateField::Windows, a.windows.clone(), b.windows.clone());
    push(
        StateField::WindowsBuild,
        a.windows_build.clone(),
        b.windows_build.clone(),
    );
    let display = |s: &SystemState| {
        s.display
            .map(|d| format!("{}×{} @ {} Гц", d.width, d.height, d.hz))
    };
    push(StateField::Display, display(a), display(b));
    push(
        StateField::PowerPlan,
        a.power_plan.clone(),
        b.power_plan.clone(),
    );
    push(StateField::Bios, a.bios.clone(), b.bios.clone());
    let ram = |s: &SystemState| s.ram_mts.map(|v| format!("{v} MT/s"));
    push(StateField::Ram, ram(a), ram(b));
    push(StateField::Hags, a.hags.clone(), b.hags.clone());
    push(
        StateField::GameMode,
        a.game_mode.clone(),
        b.game_mode.clone(),
    );
    // Файл гри: розмір або час зміни — показуємо розмір у МБ, щоб «було → стало» мало зміст.
    if let (Some(fa), Some(fb)) = (a.game_file, b.game_file) {
        if fa != fb {
            let mb = |bytes: u64| format!("{:.1} МБ", bytes as f64 / 1_048_576.0);
            out.push(StateChange {
                field: StateField::GameFile,
                from: mb(fa.bytes),
                to: mb(fb.bytes),
            });
        }
    }
    if !a.optimax.is_empty() && !b.optimax.is_empty() && a.optimax != b.optimax {
        out.push(StateChange {
            field: StateField::Optimax,
            from: a.optimax.clone(),
            to: b.optimax.clone(),
        });
    }
    out
}

/// Напрям зміни FPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    /// Помітно краще.
    Better,
    /// Помітно гірше.
    Worse,
    /// Різниця в межах похибки.
    Same,
    /// Немає FPS в одному з записів.
    Unknown,
}

/// Порівняння двох записів (старіший → новіший).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecordComparison {
    /// Старіший запис.
    pub older: HistoryEntry,
    /// Новіший запис.
    pub newer: HistoryEntry,
    /// Зміна середнього FPS, %.
    pub avg_fps_change_pct: Option<f64>,
    /// Зміна 1% low, %.
    pub low_1pct_change_pct: Option<f64>,
    /// Поріг значущості для цієї пари, %.
    pub threshold_pct: f64,
    /// Висновок за середнім FPS.
    pub trend: Trend,
    /// Та сама гра.
    pub same_game: bool,
    /// Той самий режим монітора (інакше FPS не порівнянний напряму).
    pub same_display: bool,
    /// Та сама сцена (за мітками користувача).
    pub scene: SceneMatch,
    /// Вузьке місце змінилося.
    pub bottleneck_changed: bool,
    /// Що змінилося в стані ПК.
    pub changes: Vec<StateChange>,
}

fn change_pct(from: Option<f64>, to: Option<f64>) -> Option<f64> {
    match (from, to) {
        (Some(a), Some(b)) if a > 0.0 => Some((b - a) / a * 100.0),
        _ => None,
    }
}

/// Порівнює два записи; порядок аргументів не важливий — старіший визначається за часом.
/// Поріг значущості: не менше [`MIN_SIGNIFICANT_PCT`] і не менше подвоєної сумарної похибки
/// обох записів; без оцінки похибки береться мінімальний поріг.
pub fn compare(a: &HistoryEntry, b: &HistoryEntry) -> RecordComparison {
    let (older, newer) = if a.at_unix_ms <= b.at_unix_ms {
        (a, b)
    } else {
        (b, a)
    };
    let noise = match (older.fps_noise_pct, newer.fps_noise_pct) {
        (Some(x), Some(y)) => 2.0 * (x * x + y * y).sqrt(),
        _ => 0.0,
    };
    let threshold_pct = noise.max(MIN_SIGNIFICANT_PCT);
    let avg = change_pct(older.avg_fps, newer.avg_fps);
    let trend = match avg {
        None => Trend::Unknown,
        Some(d) if d >= threshold_pct => Trend::Better,
        Some(d) if d <= -threshold_pct => Trend::Worse,
        Some(_) => Trend::Same,
    };
    RecordComparison {
        avg_fps_change_pct: avg,
        low_1pct_change_pct: change_pct(older.low_1pct_fps, newer.low_1pct_fps),
        threshold_pct,
        trend,
        same_game: older.game.eq_ignore_ascii_case(&newer.game),
        same_display: older.state.display == newer.state.display,
        scene: scene_match(older.scene.as_deref(), newer.scene.as_deref()),
        bottleneck_changed: older.bottleneck != newer.bottleneck,
        changes: diff(&older.state, &newer.state),
        older: older.clone(),
        newer: newer.clone(),
    }
}

/// Помітне погіршення в грі: найновіший запис проти попереднього порівнянного.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Regression {
    /// Порівняння (старіший — попередній порівнянний запис, новіший — останній запис гри).
    pub comparison: RecordComparison,
}

/// Чи можна порівнювати записи напряму: та сама гра й режим монітора, і сцени не різні
/// (якщо міток немає — порівнюємо, але інтерфейс про це попереджає).
fn comparable(a: &HistoryEntry, b: &HistoryEntry) -> bool {
    a.game.eq_ignore_ascii_case(&b.game)
        && a.state.display == b.state.display
        && scene_match(a.scene.as_deref(), b.scene.as_deref()) != SceneMatch::Different
}

/// Пошук погіршення: для кожної гри останній запис порівнюється з попереднім порівнянним
/// (та сама гра, режим монітора, сцена не відрізняється). Повертає ігри, де FPS помітно
/// впав (більше за поріг похибки), найсвіжіші першими. `entries` — у будь-якому порядку.
pub fn regressions(entries: &[HistoryEntry]) -> Vec<Regression> {
    let mut sorted: Vec<&HistoryEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| std::cmp::Reverse(e.at_unix_ms));
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (i, newest) in sorted.iter().enumerate() {
        let game = newest.game.to_ascii_lowercase();
        if seen.contains(&game) {
            continue;
        }
        seen.push(game);
        let previous = sorted[i + 1..].iter().find(|p| comparable(p, newest));
        if let Some(previous) = previous {
            let c = compare(previous, newest);
            if c.trend == Trend::Worse {
                out.push(Regression { comparison: c });
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{DisplayState, FileStamp};
    use optimax_core::Text;
    use optimax_diagnosis::Bottleneck;

    pub(crate) fn entry(at: i64, fps: f64, noise: Option<f64>) -> HistoryEntry {
        HistoryEntry {
            id: format!("{at}-game.exe"),
            game: "game.exe".into(),
            at_unix_ms: at,
            bottleneck: Bottleneck::Gpu,
            title: Text::new("diag.summary.gpu.title"),
            confidence: 85,
            avg_fps: Some(fps),
            low_1pct_fps: Some(fps * 0.7),
            low_01pct_fps: None,
            frames: 6000,
            counted_s: 60.0,
            fps_noise_pct: noise,
            state: SystemState {
                optimax: "0.1.6".into(),
                driver: Some("566.36".into()),
                display: Some(DisplayState {
                    width: 2560,
                    height: 1440,
                    hz: 200,
                }),
                ..Default::default()
            },
            has_recording: true,
            scene: None,
        }
    }

    #[test]
    fn noise_from_steady_and_jumpy_play() {
        // Рівні 100 FPS: розкиду немає.
        let steady = vec![10.0; 6000];
        assert!(fps_noise_pct(&steady).unwrap() < 1e-9);
        // Секунди по черзі 80 і 120 FPS: σ ≈ 20, середнє 100, 60 с → ≈ 2.6 %.
        let mut jumpy = Vec::new();
        for s in 0..60 {
            let fps = if s % 2 == 0 { 80.0 } else { 120.0 };
            jumpy.extend(std::iter::repeat_n(1000.0 / fps, fps as usize));
        }
        let n = fps_noise_pct(&jumpy).unwrap();
        assert!((2.4..2.8).contains(&n), "{n}");
        // Замало секунд — без оцінки.
        assert!(fps_noise_pct(&vec![10.0; 500]).is_none());
    }

    #[test]
    fn small_drop_is_noise_large_drop_is_worse() {
        let a = entry(1_000, 125.0, Some(1.0));
        assert_eq!(
            compare(&a, &entry(2_000, 119.0, Some(1.0))).trend,
            Trend::Same
        );
        let c = compare(&entry(2_000, 108.0, Some(1.0)), &a);
        // Порядок аргументів не важливий: старіший — за часом.
        assert_eq!(c.older.at_unix_ms, 1_000);
        assert_eq!(c.trend, Trend::Worse);
        assert!((c.avg_fps_change_pct.unwrap() + 13.6).abs() < 0.1);
        assert_eq!(c.threshold_pct, MIN_SIGNIFICANT_PCT);
        assert_eq!(compare(&a, &entry(2_000, 140.0, None)).trend, Trend::Better);
    }

    #[test]
    fn big_noise_raises_threshold() {
        // Похибка по 5 % у кожному записі → поріг 2·√50 ≈ 14.1 %: −12 % — ще в межах похибки.
        let c = compare(&entry(1, 100.0, Some(5.0)), &entry(2, 88.0, Some(5.0)));
        assert!((c.threshold_pct - 14.14).abs() < 0.01);
        assert_eq!(c.trend, Trend::Same);
    }

    #[test]
    fn regressions_use_latest_record_against_previous_comparable_one() {
        let mut a = entry(1, 125.0, Some(1.0));
        a.scene = Some("Коридор".into());
        let mut other_scene = entry(2, 200.0, Some(1.0));
        other_scene.scene = Some("Місто".into());
        let mut b = entry(3, 108.0, Some(1.0));
        b.scene = Some(" коридор ".into());
        b.state.driver = Some("572.16".into());
        let mut cs = entry(2, 300.0, None);
        cs.game = "cs2.exe".into();
        let r = regressions(&[a.clone(), other_scene.clone(), b.clone(), cs.clone()]);
        // «Місто» пропущено (інша сцена); порівняння коридор → коридор, драйвер змінився.
        assert_eq!(r.len(), 1);
        let c = &r[0].comparison;
        assert_eq!((c.older.at_unix_ms, c.newer.at_unix_ms), (1, 3));
        assert_eq!(c.scene, SceneMatch::Same);
        assert_eq!(c.changes[0].field, StateField::Driver);
        // Покращення чи зміна в межах похибки — не погіршення.
        let mut better = b.clone();
        better.avg_fps = Some(150.0);
        assert!(regressions(&[a.clone(), better]).is_empty());
        // Інший режим монітора — не порівнюємо.
        let mut other_display = b;
        other_display.state.display = None;
        assert!(regressions(&[a, other_display]).is_empty());
        // Без попереднього запису — нічого.
        assert!(regressions(&[cs]).is_empty());
    }

    #[test]
    fn scene_labels() {
        use crate::model::{clean_scene, scene_match};
        assert_eq!(clean_scene(Some("  Коридор\n ")), Some("Коридор".into()));
        assert_eq!(clean_scene(Some("   ")), None);
        assert_eq!(
            clean_scene(Some(&"x".repeat(100))).map(|s| s.len()),
            Some(40)
        );
        assert_eq!(scene_match(Some("A"), Some("a ")), SceneMatch::Same);
        assert_eq!(scene_match(Some("A"), Some("B")), SceneMatch::Different);
        assert_eq!(scene_match(Some("A"), None), SceneMatch::Unknown);
    }

    #[test]
    fn diff_lists_only_known_changes() {
        let a = entry(1, 100.0, None);
        let mut b = entry(2, 100.0, None);
        b.state.driver = Some("572.16".into());
        b.state.power_plan = Some("Збалансований".into()); // було невідомо — не зміна
        b.state.display = Some(DisplayState {
            width: 1920,
            height: 1080,
            hz: 144,
        });
        b.state.optimax = "0.1.7".into();
        let mut a2 = a.clone();
        a2.state.game_file = Some(FileStamp {
            bytes: 100 * 1_048_576,
            modified_unix: 1,
        });
        b.state.game_file = Some(FileStamp {
            bytes: 101 * 1_048_576,
            modified_unix: 2,
        });
        let c = compare(&a2, &b);
        let fields: Vec<StateField> = c.changes.iter().map(|x| x.field).collect();
        assert_eq!(
            fields,
            vec![
                StateField::Driver,
                StateField::Display,
                StateField::GameFile,
                StateField::Optimax
            ]
        );
        assert_eq!(c.changes[0].from, "566.36");
        assert_eq!(c.changes[0].to, "572.16");
        assert_eq!(c.changes[1].to, "1920×1080 @ 144 Гц");
        assert_eq!(c.changes[2].from, "100.0 МБ");
        assert!(!c.same_display);
        assert!(diff(&a.state, &a.state).is_empty());
    }
}
