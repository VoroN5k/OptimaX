//! Рушій експериментів: чесне порівняння «як було» (A) і «зі зміною» (B) у грі.
//!
//! - Порядок прогонів ABBA ABBA… ([`next_arm`]): лінійний дрейф (прогрів, підвантаження
//!   шейдерів) рівно лягає на обидва варіанти.
//! - Ефект — з моделі «рівень + варіант + дрейф за номером прогону» ([`stats::fit`]), тож
//!   дрейф не приписується зміні; інтервал — за Стьюдентом.
//! - Кількість прогонів адаптивна (від [`Policy::min_per_arm`] до [`Policy::max_per_arm`]):
//!   після кожної повної пари — перевірка, чи висновок уже певний. Щоб повторні перевірки не
//!   завищували шанс хибного висновку, на кожній береться суворіший рівень (межа Покока для
//!   шести перевірок, [`POCOCK_ALPHA`]).
//! - Висновок має три чесні варіанти: стало краще / гірше (інтервал не перетинає нуль і
//!   ефект не менший за поріг), ефекту немає (увесь інтервал у межах ±порогу) або «не ясно»
//!   (даних замало — так і кажемо).
//! - Прогони, що відрізняються від решти свого варіанта рівнем FPS чи формою розподілу кадрів
//!   (схоже на іншу сцену), позначаються ([`scene`]); виключає їх лише користувач.
//!
//! Лише чисті функції над даними — усе покрито тестами.

#![forbid(unsafe_code)]

pub mod scene;
pub mod stats;
pub mod thermal;

use optimax_core::Text;
use serde::{Deserialize, Serialize};
use stats::{fit, t_quantile, SplitMix};

/// Номінальний рівень значущості однієї перевірки: межа Покока для 6 перевірок (n = 3…8 на
/// варіант) при загальних 5 % (z = 2,453).
pub const POCOCK_ALPHA: f64 = 0.0142;
/// Менше кадрів у прогоні — замало для надійного 1% low.
pub const MIN_FRAMES_PER_RUN: usize = 1000;
/// Дрейф FPS за прогін, з якого попереджаємо (схоже, система ще прогрівається), %.
const DRIFT_WARN_PCT: f64 = 1.0;
/// Різниця середніх температур GPU між варіантами, після якої умови різні, °C.
const TEMP_DRIFT_C: f64 = 5.0;
/// Розбіжність тривалостей прогонів, після якої попереджаємо, частка.
const DURATION_MISMATCH: f64 = 0.10;
/// Точність 1% low одного прогону (половина 95 % інтервалу), гірше за яку радимо довші
/// прогони, %.
const LOW_PRECISION_WARN_PCT: f64 = 5.0;
/// Повторів bootstrap для точності 1% low одного прогону.
const BOOTSTRAP_ROUNDS: usize = 200;
/// Довжина блоку кадрів для bootstrap (сусідні кадри залежні).
const BOOTSTRAP_BLOCK: usize = 128;

/// Варіант: A — як було, B — зі зміною.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Arm {
    /// Як було.
    A,
    /// Зі зміною.
    B,
}

/// Правила тесту.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Policy {
    /// Найменше прогонів на варіант до першого висновку.
    pub min_per_arm: usize,
    /// Найбільше прогонів на варіант.
    pub max_per_arm: usize,
    /// Найменший ефект на середній FPS, який вважаємо помітним, %.
    pub margin_avg_pct: f64,
    /// Те саме для 1% low (він шумніший), %.
    pub margin_low_pct: f64,
    /// Рівень значущості однієї перевірки.
    pub alpha: f64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            min_per_arm: 3,
            max_per_arm: 8,
            margin_avg_pct: 1.0,
            margin_low_pct: 2.0,
            alpha: POCOCK_ALPHA,
        }
    }
}

/// Один прогін (вхід рушія).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// Варіант.
    pub arm: Arm,
    /// Frametime кожного кадру головного swap chain, мс.
    pub frametimes_ms: Vec<f64>,
    /// Середня температура GPU, °C.
    pub gpu_temp_avg_c: Option<f64>,
    /// FPS упирався в ліміт кадрів (зміна могла не проявитися).
    pub frame_capped: bool,
    /// Користувач виключив прогін (напр. після позначки «інша сцена»).
    pub excluded: bool,
}

/// Чому прогін схожий на іншу сцену.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Suspect {
    /// Середній FPS помітно інший, ніж у решти прогонів цього варіанта.
    Level,
    /// Інша форма розподілу кадрів (інша частка довгих кадрів).
    Shape,
}

/// Підсумок одного прогону.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunSummary {
    /// Варіант.
    pub arm: Arm,
    /// Номер у порядку виконання (з нуля).
    pub order: u32,
    /// Середній FPS.
    pub avg_fps: f64,
    /// 1% low.
    pub low_1pct_fps: f64,
    /// Кадрів.
    pub frames: u32,
    /// Тривалість, с.
    pub duration_s: f64,
    /// Точність 1% low цього прогону (половина 95 % інтервалу, блочний bootstrap), %.
    pub low_precision_pct: Option<f64>,
    /// Виключено користувачем.
    pub excluded: bool,
    /// Схоже на іншу сцену.
    pub suspect: Option<Suspect>,
}

/// Висновок за однією метрикою.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Стало краще.
    Better,
    /// Стало гірше.
    Worse,
    /// Ефекту немає: увесь інтервал у межах ±порогу.
    NoEffect,
    /// Даних замало для висновку.
    Unclear,
}

/// Оцінка ефекту за однією метрикою.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Estimate {
    /// Рівень A в середній момент тесту (без дрейфу).
    pub a: f64,
    /// Рівень B у той самий момент.
    pub b: f64,
    /// Ефект B − A, % від A.
    pub delta_pct: f64,
    /// Нижня межа інтервалу, %.
    pub ci_low_pct: f64,
    /// Верхня межа інтервалу, %.
    pub ci_high_pct: f64,
    /// Дрейф за один прогін, % від A.
    pub drift_pct_per_run: f64,
    /// Розкид одного прогону навколо моделі, % від A.
    pub run_sd_pct: f64,
    /// Висновок.
    pub outcome: Outcome,
}

/// Стан тесту.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Потрібні ще прогони ([`Report::next`]).
    Continue,
    /// Висновок певний або прогонів досить.
    Done,
}

/// Звіт рушія після чергового прогону.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Report {
    /// Продовжувати чи вже все.
    pub status: Status,
    /// Який варіант запускати наступним.
    pub next: Option<Arm>,
    /// Загальний висновок (`Unclear`, поки тест іде чи даних замало).
    pub verdict: Outcome,
    /// Середній FPS (немає — замало прогонів).
    pub avg_fps: Option<Estimate>,
    /// 1% low.
    pub low_1pct: Option<Estimate>,
    /// Скільки ще пар прогонів орієнтовно треба для висновку за середнім FPS.
    pub more_pairs: Option<u32>,
    /// Рівень довіри інтервалів, %.
    pub confidence_pct: f64,
    /// Прогони в порядку виконання.
    pub runs: Vec<RunSummary>,
    /// Попередження про умови тесту.
    pub warnings: Vec<Text>,
}

/// Наступний варіант за порядком ABBA ABBA… (рахуються лише невиключені прогони; повтор
/// виключеного прогону відновлює рівновагу).
pub fn next_arm(runs: &[Run]) -> Arm {
    let a = runs
        .iter()
        .filter(|r| !r.excluded && r.arm == Arm::A)
        .count();
    let b = runs
        .iter()
        .filter(|r| !r.excluded && r.arm == Arm::B)
        .count();
    match a.cmp(&b) {
        std::cmp::Ordering::Less => Arm::A,
        std::cmp::Ordering::Greater => Arm::B,
        // Пара k: парна — AB, непарна — BA.
        std::cmp::Ordering::Equal if a % 2 == 0 => Arm::A,
        std::cmp::Ordering::Equal => Arm::B,
    }
}

/// Середнє найгірших 1 % frametime → FPS (як у `optimax_frametime::frame_stats`).
fn low1_fps(ft: &mut [f64]) -> f64 {
    let n = ft.len();
    if n == 0 {
        return 0.0;
    }
    let k = ((n as f64 * 0.01).ceil() as usize).clamp(1, n);
    // Після розбиття на місці n − k стоїть k-й найдовший кадр, правіше — довші за нього.
    ft.select_nth_unstable_by(n - k, f64::total_cmp);
    let mean = ft[n - k..].iter().sum::<f64>() / k as f64;
    if mean > 0.0 {
        1000.0 / mean
    } else {
        0.0
    }
}

/// Точність 1% low одного прогону: половина 95 % інтервалу за блочним bootstrap, % від
/// значення. `None` — замало кадрів.
pub fn low_precision_pct(frametimes_ms: &[f64], seed: u64) -> Option<f64> {
    let n = frametimes_ms.len();
    if n < BOOTSTRAP_BLOCK * 4 {
        return None;
    }
    let base = low1_fps(&mut frametimes_ms.to_vec());
    if base <= 0.0 {
        return None;
    }
    let blocks = n.div_ceil(BOOTSTRAP_BLOCK);
    let mut rng = SplitMix::new(seed);
    let mut sample = Vec::with_capacity(blocks * BOOTSTRAP_BLOCK);
    let mut values = Vec::with_capacity(BOOTSTRAP_ROUNDS);
    for _ in 0..BOOTSTRAP_ROUNDS {
        sample.clear();
        for _ in 0..blocks {
            let start = rng.below(n - BOOTSTRAP_BLOCK + 1);
            sample.extend_from_slice(&frametimes_ms[start..start + BOOTSTRAP_BLOCK]);
        }
        sample.truncate(n);
        values.push(low1_fps(&mut sample));
    }
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    Some((at(0.975) - at(0.025)) / 2.0 / base * 100.0)
}

fn summarize(order: usize, r: &Run) -> RunSummary {
    let stats = optimax_frametime::frame_stats(&r.frametimes_ms);
    RunSummary {
        arm: r.arm,
        order: u32::try_from(order).unwrap_or(u32::MAX),
        avg_fps: stats.as_ref().map_or(0.0, |s| s.avg_fps),
        low_1pct_fps: stats.as_ref().map_or(0.0, |s| s.low_1pct_fps),
        frames: u32::try_from(r.frametimes_ms.len()).unwrap_or(u32::MAX),
        duration_s: stats.as_ref().map_or(0.0, |s| s.duration_ms / 1000.0),
        low_precision_pct: low_precision_pct(&r.frametimes_ms, 0x5EED ^ order as u64),
        excluded: r.excluded,
        suspect: None,
    }
}

fn classify(delta: f64, lo: f64, hi: f64, margin: f64) -> Outcome {
    if lo > 0.0 && delta >= margin {
        Outcome::Better
    } else if hi < 0.0 && delta <= -margin {
        Outcome::Worse
    } else if lo > -margin && hi < margin {
        Outcome::NoEffect
    } else {
        Outcome::Unclear
    }
}

fn estimate(
    runs: &[RunSummary],
    value: fn(&RunSummary) -> f64,
    margin: f64,
    alpha: f64,
) -> Option<Estimate> {
    let points: Vec<(bool, f64, f64)> = runs
        .iter()
        .filter(|r| !r.excluded)
        .map(|r| (r.arm == Arm::B, f64::from(r.order), value(r)))
        .collect();
    let f = fit(&points)?;
    if f.level_a <= 0.0 {
        return None;
    }
    let half = t_quantile(1.0 - alpha / 2.0, f.df) * f.effect_se;
    let pct = |v: f64| v / f.level_a * 100.0;
    let (delta, lo, hi) = (pct(f.effect), pct(f.effect - half), pct(f.effect + half));
    Some(Estimate {
        a: f.level_a,
        b: f.level_a + f.effect,
        delta_pct: delta,
        ci_low_pct: lo,
        ci_high_pct: hi,
        drift_pct_per_run: pct(f.drift),
        run_sd_pct: pct(f.residual_sd),
        outcome: classify(delta, lo, hi, margin),
    })
}

/// Скільки ще пар прогонів орієнтовно треба, щоб інтервал середнього FPS став досить вузьким
/// для висновку (за поточним розкидом). `None` — висновок уже є.
fn more_pairs(e: &Estimate, per_arm: usize, margin: f64, alpha: f64) -> Option<u32> {
    if e.outcome != Outcome::Unclear {
        return None;
    }
    let d = e.delta_pct.abs();
    // Для «є ефект» інтервал має не зачепити нуль; для «немає» — вміститися в ±поріг.
    let need_half = if d >= margin { d } else { margin - d };
    let need_half = need_half.max(margin * 0.25);
    let z = t_quantile(
        1.0 - alpha / 2.0,
        (2 * per_arm).saturating_sub(3).max(3) as f64,
    );
    // Половина інтервалу ≈ z·σ·√(2/n) за рівних груп.
    let n_needed = 2.0 * (z * e.run_sd_pct / need_half).powi(2);
    let more = (n_needed - per_arm as f64).ceil().max(1.0);
    Some(more.min(f64::from(u16::MAX)) as u32)
}

/// Оцінює прогони: чи вже є висновок, і якщо ні — який варіант запускати далі.
pub fn evaluate(runs: &[Run], policy: &Policy) -> Report {
    let mut summaries: Vec<RunSummary> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| summarize(i, r))
        .collect();
    scene::mark_suspects(runs, &mut summaries);
    let count = |arm| {
        summaries
            .iter()
            .filter(|r| !r.excluded && r.arm == arm)
            .count()
    };
    let (na, nb) = (count(Arm::A), count(Arm::B));
    let balanced = na == nb;
    let enough = na.min(nb) >= policy.min_per_arm;
    let (avg, low) = if enough {
        (
            estimate(
                &summaries,
                |r| r.avg_fps,
                policy.margin_avg_pct,
                policy.alpha,
            ),
            estimate(
                &summaries,
                |r| r.low_1pct_fps,
                policy.margin_low_pct,
                policy.alpha,
            ),
        )
    } else {
        (None, None)
    };
    let decided = match (&avg, &low) {
        (Some(a), Some(l)) => {
            a.outcome != Outcome::Unclear
                // 1% low шумніший: його «не ясно» не тримає тест, якщо оцінка не вказує на
                // погіршення (тоді збираємо ще прогони, щоб його підтвердити чи зняти).
                && (l.outcome != Outcome::Unclear || l.delta_pct > -policy.margin_low_pct)
        }
        _ => false,
    };
    let at_max = na.min(nb) >= policy.max_per_arm;
    let done = balanced && enough && (decided || at_max);
    let verdict = if done {
        overall(avg.as_ref(), low.as_ref())
    } else {
        Outcome::Unclear
    };
    let more = avg
        .as_ref()
        .and_then(|e| more_pairs(e, na.min(nb), policy.margin_avg_pct, policy.alpha))
        .filter(|_| !done);
    Report {
        status: if done { Status::Done } else { Status::Continue },
        next: (!done).then(|| next_arm(runs)),
        verdict,
        warnings: warnings(runs, &summaries, avg.as_ref()),
        avg_fps: avg,
        low_1pct: low,
        more_pairs: more,
        confidence_pct: (1.0 - policy.alpha) * 100.0,
        runs: summaries,
    }
}

fn overall(avg: Option<&Estimate>, low: Option<&Estimate>) -> Outcome {
    let (Some(a), Some(l)) = (avg, low) else {
        return Outcome::Unclear;
    };
    if a.outcome == Outcome::Worse || l.outcome == Outcome::Worse {
        Outcome::Worse
    } else if a.outcome == Outcome::Better
        || (a.outcome == Outcome::NoEffect && l.outcome == Outcome::Better)
    {
        Outcome::Better
    } else if a.outcome == Outcome::NoEffect {
        Outcome::NoEffect
    } else {
        Outcome::Unclear
    }
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = values.collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn warnings(runs: &[Run], sums: &[RunSummary], avg: Option<&Estimate>) -> Vec<Text> {
    let mut out = Vec::new();
    let included = || runs.iter().zip(sums).filter(|(r, _)| !r.excluded);
    let temp = |arm| {
        mean(
            included()
                .filter(|(r, _)| r.arm == arm)
                .filter_map(|(r, _)| r.gpu_temp_avg_c),
        )
    };
    if let (Some(ta), Some(tb)) = (temp(Arm::A), temp(Arm::B)) {
        if (ta - tb).abs() > TEMP_DRIFT_C {
            out.push(
                Text::new("exp.warn.temp_drift")
                    .arg("a", format!("{ta:.0}"))
                    .arg("b", format!("{tb:.0}")),
            );
        }
    }
    if let Some(e) = avg {
        if e.drift_pct_per_run.abs() >= DRIFT_WARN_PCT {
            out.push(
                Text::new("exp.warn.drift").arg("pct", format!("{:+.1}", e.drift_pct_per_run)),
            );
        }
    }
    if included().any(|(r, _)| r.frame_capped) {
        out.push(Text::new("exp.warn.capped"));
    }
    if included().any(|(_, s)| (s.frames as usize) < MIN_FRAMES_PER_RUN) {
        out.push(Text::new("exp.warn.few_frames").arg("min", MIN_FRAMES_PER_RUN));
    }
    let (dmin, dmax) = included()
        .map(|(_, s)| s.duration_s)
        .fold((f64::MAX, 0.0f64), |(lo, hi), d| (lo.min(d), hi.max(d)));
    if dmax > 0.0 && (dmax - dmin) / dmax > DURATION_MISMATCH {
        out.push(
            Text::new("exp.warn.duration_mismatch")
                .arg("min", format!("{dmin:.0}"))
                .arg("max", format!("{dmax:.0}")),
        );
    }
    let worst_precision = included()
        .filter_map(|(_, s)| s.low_precision_pct)
        .fold(0.0f64, f64::max);
    if worst_precision > LOW_PRECISION_WARN_PCT {
        out.push(Text::new("exp.warn.low_precision").arg("pct", format!("{worst_precision:.0}")));
    }
    for s in sums.iter().filter(|s| !s.excluded) {
        if let Some(why) = s.suspect {
            let key = match why {
                Suspect::Level => "exp.warn.suspect_level",
                Suspect::Shape => "exp.warn.suspect_shape",
            };
            out.push(
                Text::new(key)
                    .arg("n", s.order + 1)
                    .arg("arm", if s.arm == Arm::A { "A" } else { "B" }),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests;
