//! Правила вердиктів. Кожне — чиста функція над [`Analysis`], повертає знахідку з
//! впевненістю й доказами або `None`, якщо ознак недостатньо. Пороги — з CLAUDE.md, розд. 9.

use crate::timeline::{SamplePoint, Timeline};
use optimax_core::{Category, Finding, FindingBuilder, Impact, Text};
use optimax_frametime::{find_spikes, FrameStats};
use optimax_metrics::ThrottleReason;
use std::collections::HashMap;

/// GPU вважається «в стелі» від цього завантаження.
pub const GPU_BOUND_UTIL: u32 = 95;
/// Нижче цього середнього завантаження GPU не є вузьким місцем.
pub const GPU_NOT_BOUND_AVG: f64 = 90.0;
/// Ядро вважається насиченим від цього завантаження.
pub const CORE_SATURATED_PCT: f32 = 95.0;
/// Частка VRAM, з якої пам'ять вважається майже заповненою.
pub const VRAM_FULL_FRACTION: f64 = 0.90;
/// Пів-ширина вікна кореляції навколо піку.
pub const WINDOW_MS: f64 = 200.0;
/// Частка часу, після якої ознака вважається постійною.
const MOSTLY: f64 = 0.6;
/// Відсічка частоти: нижче цієї частки від максимуму високе завантаження GPU означає
/// «легке навантаження на зниженій частоті», а не стелю.
const GPU_CLOCK_MIN_FRACTION: f64 = 0.6;
/// Фоновий процес помітний від цієї частки всього CPU.
pub const BACKGROUND_CPU_PCT: f32 = 5.0;
/// Мінімальна активність диска, яку варто пов'язувати з фризом.
const DISK_BURST_MIN_BYTES: f64 = 20.0 * 1024.0 * 1024.0;

/// Ознака, яку перевіряємо в кожному знімку.
type SamplePredicate = Box<dyn Fn(&SamplePoint) -> bool>;

fn draft(id: &'static str, category: Category, impact: Impact, confidence: u8) -> FindingBuilder {
    FindingBuilder::new("diag", id, category, impact, confidence)
}

fn pct(v: f64) -> Text {
    Text::new("value.pct").arg("v", format!("{:.0}", v * 100.0))
}

fn conf(v: f64) -> u8 {
    v.round().clamp(0.0, 100.0) as u8
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

/// Попередньо обчислені величини, спільні для всіх правил.
pub struct Analysis<'a> {
    /// Шкала часу.
    pub timeline: &'a Timeline,
    /// Статистика кадрів.
    pub stats: FrameStats,
    /// Час піків frametime, мс.
    pub spike_times: Vec<f64>,
    /// Частота монітора, Гц (якщо відома).
    pub display_hz: Option<u32>,
}

impl<'a> Analysis<'a> {
    /// Рахує статистику й піки. `None`, якщо кадрів немає.
    pub fn new(timeline: &'a Timeline, display_hz: Option<u32>) -> Option<Self> {
        let ft: Vec<f64> = timeline.frames.iter().map(|f| f.frametime_ms).collect();
        let stats = optimax_frametime::frame_stats(&ft)?;
        let spike_times = find_spikes(&ft)
            .iter()
            .map(|s| timeline.frames[s.index].t_ms)
            .collect();
        Some(Self {
            timeline,
            stats,
            spike_times,
            display_hz,
        })
    }

    fn samples(&self) -> &[SamplePoint] {
        &self.timeline.samples
    }

    /// Частка знімків, для яких виконується умова.
    fn share(&self, pred: impl Fn(&SamplePoint) -> bool) -> f64 {
        let s = self.samples();
        if s.is_empty() {
            return 0.0;
        }
        s.iter().filter(|p| pred(p)).count() as f64 / s.len() as f64
    }

    fn gpu_avg_util(&self) -> Option<f64> {
        let v: Vec<f64> = self
            .samples()
            .iter()
            .filter_map(|s| s.gpu.as_ref()?.util.map(f64::from))
            .collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    }

    /// Медіана частки часу кадру, яку займала робота GPU / CPU (дані PresentMon 2.x).
    fn busy_ratio(&self, gpu: bool) -> Option<f64> {
        median(
            self.timeline
                .frames
                .iter()
                .filter_map(|f| {
                    let busy = if gpu { f.gpu_busy_ms } else { f.cpu_busy_ms }?;
                    (f.frametime_ms > 0.0).then(|| (busy / f.frametime_ms).min(1.5))
                })
                .collect(),
        )
    }

    /// Частка піків, у вікні ±200 мс яких була ознака, і «фонова» частка таких самих вікон
    /// навколо кожного знімка. Ознака пов'язана з фризами, якщо перша помітно більша.
    fn coincidence(&self, pred: &dyn Fn(&SamplePoint) -> bool) -> (f64, f64) {
        let in_window = |t: f64| self.timeline.samples_near(t, WINDOW_MS).iter().any(pred);
        let spikes = &self.spike_times;
        let at_spikes = if spikes.is_empty() {
            0.0
        } else {
            spikes.iter().filter(|t| in_window(**t)).count() as f64 / spikes.len() as f64
        };
        let samples = self.samples();
        let baseline = if samples.is_empty() {
            0.0
        } else {
            samples.iter().filter(|s| in_window(s.t_ms)).count() as f64 / samples.len() as f64
        };
        (at_spikes, baseline)
    }
}

/// Стабільний ліміт кадрів (V-Sync, обмежувач FPS): frametime майже не коливається,
/// а GPU не в стелі. У такому разі CPU/GPU не є вузьким місцем — FPS обмежено навмисно.
pub fn frame_cap(a: &Analysis<'_>) -> Option<Finding> {
    let mut ft: Vec<f64> = a.timeline.frames.iter().map(|f| f.frametime_ms).collect();
    ft.sort_by(f64::total_cmp);
    let p = |q: f64| ft[((ft.len() - 1) as f64 * q).round() as usize];
    let (p5, p95) = (p(0.05), p(0.95));
    let stable = p5 > 0.0 && p95 / p5 <= 1.15;
    let gpu_free = a.gpu_avg_util().is_none_or(|u| u < GPU_NOT_BOUND_AVG);
    if !stable || !gpu_free {
        return None;
    }
    let fps = 1000.0 / a.stats.median_frametime_ms;
    let near_refresh = a
        .display_hz
        .filter(|hz| *hz > 0)
        .is_some_and(|hz| (fps - f64::from(hz)).abs() / f64::from(hz) <= 0.03);
    let core_busy = a.share(|s| s.busiest_core_pct >= CORE_SATURATED_PCT);
    let mut d = if near_refresh {
        draft("cap.vsync", Category::FrameCap, Impact::Info, 85)
            .arg("hz", a.display_hz.unwrap_or_default())
    } else {
        draft("cap.limiter", Category::FrameCap, Impact::Info, 70)
    }
    .arg("fps", format!("{fps:.0}"))
    .evidence(
        Text::new("diag.ev.frametime_spread"),
        Text::new("diag.ev.ms_range")
            .arg("lo", format!("{p5:.2}"))
            .arg("hi", format!("{p95:.2}")),
    );
    if let Some(u) = a.gpu_avg_util() {
        d = d.evidence(
            Text::new("diag.ev.gpu_avg"),
            Text::new("value.pct").arg("v", format!("{u:.0}")),
        );
    }
    if core_busy >= 0.5 {
        // Ядро в стелі при стабільному ліміті — найімовірніше, цикл очікування кадру.
        d = d.evidence(Text::new("diag.ev.core_busy_wait"), pct(core_busy));
    }
    Some(d.build())
}

/// GPU-bound: GPU ≥ 95 % більшу частину часу на робочій частоті.
pub fn gpu_bound(a: &Analysis<'_>) -> Option<Finding> {
    let busy = |s: &SamplePoint| {
        let Some(g) = &s.gpu else { return false };
        let clock_ok = match (g.clock_mhz, g.max_clock_mhz) {
            (Some(c), Some(m)) if m > 0 => f64::from(c) >= f64::from(m) * GPU_CLOCK_MIN_FRACTION,
            _ => true,
        };
        g.util.is_some_and(|u| u >= GPU_BOUND_UTIL) && clock_ok
    };
    let share = a.share(busy);
    let ratio = a.busy_ratio(true);
    let ratio_high = ratio.is_some_and(|r| r >= 0.9);
    if !(share >= MOSTLY || (share >= 0.4 && ratio_high)) {
        return None;
    }
    // Чутливість FPS до роздільності не перевірялась (потрібен окремий прогін) — стеля 85 %.
    let confidence = (50.0 + 30.0 * share + if ratio_high { 10.0 } else { 0.0 }).min(85.0);
    let mut d = draft("gpu_bound", Category::Gpu, Impact::Info, conf(confidence))
        .arg("share", format!("{:.0}", share * 100.0))
        .advice()
        .evidence(Text::new("diag.ev.gpu_busy_share"), pct(share));
    if let Some(u) = a.gpu_avg_util() {
        d = d.evidence(
            Text::new("diag.ev.gpu_avg"),
            Text::new("value.pct").arg("v", format!("{u:.0}")),
        );
    }
    if let Some(r) = ratio {
        d = d.evidence(Text::new("diag.ev.gpu_busy_ratio"), pct(r.min(1.0)));
    }
    let power_cap = a.share(|s| {
        s.gpu
            .as_ref()
            .is_some_and(|g| g.throttle.contains(&ThrottleReason::SwPowerCap))
    });
    if power_cap >= 0.5 {
        d = d.evidence(Text::new("diag.ev.power_cap_normal"), pct(power_cap));
    }
    Some(d.build())
}

/// CPU-bound: GPU < 90 %, а найзавантаженіше ядро ≥ 95 % більшу частину часу.
/// Аналізується **кожне** ядро в кожен момент: головний потік гри мігрує між ядрами.
pub fn cpu_bound(a: &Analysis<'_>) -> Option<Finding> {
    let gpu_avg = a.gpu_avg_util()?;
    if gpu_avg >= GPU_NOT_BOUND_AVG {
        return None;
    }
    let share = a.share(|s| s.busiest_core_pct >= CORE_SATURATED_PCT);
    if share < 0.5 {
        return None;
    }
    let ratio = a.busy_ratio(false);
    let ratio_high = ratio.is_some_and(|r| r >= 0.8);
    let confidence = (45.0 + 35.0 * share + if ratio_high { 10.0 } else { 0.0 }).min(85.0);

    let mut per_core: HashMap<usize, usize> = HashMap::new();
    for s in a
        .samples()
        .iter()
        .filter(|s| s.busiest_core_pct >= CORE_SATURATED_PCT)
    {
        *per_core.entry(s.busiest_core).or_default() += 1;
    }
    let distinct_cores = per_core.len();
    let top_core = per_core
        .iter()
        .max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0)))
        .map(|(c, _)| *c)
        .unwrap_or_default();

    let mut d = draft(
        "cpu_bound",
        Category::Cpu,
        Impact::Moderate,
        conf(confidence),
    )
    .arg("share", format!("{:.0}", share * 100.0))
    .arg("gpu", format!("{gpu_avg:.0}"))
    .arg("core", top_core)
    .advice()
    .evidence(Text::new("diag.ev.core_saturated_share"), pct(share))
    .evidence(
        Text::new("diag.ev.gpu_avg"),
        Text::new("value.pct").arg("v", format!("{gpu_avg:.0}")),
    )
    .evidence(
        Text::new("diag.ev.cores_hopping"),
        Text::raw(distinct_cores),
    );
    if let Some(r) = ratio {
        d = d.evidence(Text::new("diag.ev.cpu_busy_ratio"), pct(r.min(1.0)));
    }
    Some(d.build())
}

/// Відеопам'ять майже заповнена. Shared-пам'ять (вихід за VRAM у RAM) поки не вимірюється,
/// тому сильний висновок — лише якщо піки збігаються з моментами заповнення.
pub fn vram_pressure(a: &Analysis<'_>) -> Option<Finding> {
    let full = |s: &SamplePoint| {
        s.gpu
            .as_ref()
            .and_then(|g| g.vram_fraction())
            .is_some_and(|f| f >= VRAM_FULL_FRACTION)
    };
    let share = a.share(full);
    if share < 0.3 {
        return None;
    }
    let peak = a
        .samples()
        .iter()
        .filter_map(|s| s.gpu.as_ref()?.vram_fraction())
        .fold(0.0, f64::max);
    let (at_spikes, base) = a.coincidence(&full);
    // VRAM буває повною довго, тож вимагаємо збіг у ≥ 1.5 раза частіший за звичайний.
    let correlated = a.spike_times.len() >= 3 && at_spikes >= 0.5 && at_spikes >= base * 1.5;
    let d = if correlated {
        draft("vram.stutter", Category::Vram, Impact::High, 70)
    } else {
        draft("vram.near_full", Category::Vram, Impact::Moderate, 50)
    };
    Some(
        d.arg("peak", format!("{:.0}", peak * 100.0))
            .advice()
            .evidence(Text::new("diag.ev.vram_full_share"), pct(share))
            .evidence(Text::new("diag.ev.vram_peak"), pct(peak))
            .evidence(Text::new("diag.ev.spikes_with_factor"), pct(at_spikes))
            .build(),
    )
}

const THERMAL: [ThrottleReason; 4] = [
    ThrottleReason::SwThermal,
    ThrottleReason::HwThermal,
    ThrottleReason::HwSlowdown,
    ThrottleReason::HwPowerBrake,
];

/// Тепловий / апаратний тротлінг GPU за прямими причинами NVML.
/// Ліміт потужності (SwPowerCap) — штатний режим під навантаженням, тут не рахується.
/// Тротлінг CPU поки не вимірюється (немає надійної температури CPU без драйвера).
pub fn throttling(a: &Analysis<'_>) -> Option<Finding> {
    let active = |s: &SamplePoint| {
        s.gpu.as_ref().is_some_and(|g| {
            g.util.is_none_or(|u| u >= 50) && g.throttle.iter().any(|r| THERMAL.contains(r))
        })
    };
    let share = a.share(active);
    if share < 0.1 {
        return None;
    }
    let mut reasons: Vec<&'static str> = Vec::new();
    for s in a.samples() {
        for r in s.gpu.iter().flat_map(|g| g.throttle.iter()) {
            let key = match r {
                ThrottleReason::SwThermal => "diag.reason.sw_thermal",
                ThrottleReason::HwThermal => "diag.reason.hw_thermal",
                ThrottleReason::HwSlowdown => "diag.reason.hw_slowdown",
                ThrottleReason::HwPowerBrake => "diag.reason.hw_power_brake",
                _ => continue,
            };
            if !reasons.contains(&key) {
                reasons.push(key);
            }
        }
    }
    let max_temp = a
        .samples()
        .iter()
        .filter_map(|s| s.gpu.as_ref()?.temperature_c)
        .max();
    let (at_spikes, _) = a.coincidence(&active);
    let mut d = draft("throttling", Category::Throttling, Impact::High, 80)
        .arg("share", format!("{:.0}", share * 100.0))
        .advice()
        .evidence(Text::new("diag.ev.throttle_share"), pct(share))
        .evidence(Text::new("diag.ev.spikes_with_factor"), pct(at_spikes));
    if let Some(t) = max_temp {
        d = d.evidence(
            Text::new("diag.ev.gpu_max_temp"),
            Text::new("value.celsius").arg("v", t),
        );
    }
    for r in reasons {
        d = d.evidence(Text::new("diag.ev.reason"), Text::new(r));
    }
    Some(d.build())
}

/// Можлива причина фризів і перевірка, чи вона справді частіше трапляється біля піків.
struct Cause {
    key: &'static str,
    at_spikes: f64,
}

fn disk_threshold(a: &Analysis<'_>) -> f64 {
    let med = median(
        a.samples()
            .iter()
            .map(|s| s.disk_bytes_per_sec as f64)
            .collect(),
    )
    .unwrap_or(0.0);
    (med * 3.0).max(DISK_BURST_MIN_BYTES)
}

/// Медіанне навантаження кожного фонового процесу (для виявлення сплесків).
fn background_medians(a: &Analysis<'_>) -> HashMap<String, f32> {
    let mut all: HashMap<&str, Vec<f64>> = HashMap::new();
    for s in a.samples() {
        for name in s.background.keys() {
            all.entry(name).or_default();
        }
    }
    for s in a.samples() {
        for (name, v) in all.iter_mut() {
            v.push(f64::from(s.background.get(*name).copied().unwrap_or(0.0)));
        }
    }
    all.into_iter()
        .map(|(n, v)| (n.to_string(), median(v).unwrap_or(0.0) as f32))
        .collect()
}

/// Мікрофризи: 1% low значно нижчий за середній FPS або часті піки.
/// Причину шукаємо у вікні ±200 мс навколо піків і приписуємо, лише якщо ознака там
/// трапляється щонайменше у половині піків і вдвічі частіше, ніж у звичайні моменти.
pub fn stutter(a: &Analysis<'_>) -> Option<Finding> {
    let s = &a.stats;
    let ratio = if s.avg_fps > 0.0 {
        s.low_1pct_fps / s.avg_fps
    } else {
        1.0
    };
    let spike_rate = s.spikes as f64 / s.frames.max(1) as f64;
    let significant = (ratio < 0.5 && s.spikes >= 3) || (spike_rate >= 0.01 && s.spikes >= 5);
    if !significant {
        return isolated_hitches(a);
    }

    let disk_limit = disk_threshold(a);
    let medians = background_medians(a);
    let checks: [(&'static str, SamplePredicate); 5] = [
        (
            "diag.cause.disk",
            Box::new(move |p: &SamplePoint| p.disk_bytes_per_sec as f64 >= disk_limit),
        ),
        (
            "diag.cause.background",
            Box::new(move |p: &SamplePoint| {
                p.background.iter().any(|(n, v)| {
                    let m = medians.get(n).copied().unwrap_or(0.0);
                    *v >= BACKGROUND_CPU_PCT && *v >= m * 3.0
                })
            }),
        ),
        (
            "diag.cause.vram",
            Box::new(|p: &SamplePoint| {
                p.gpu
                    .as_ref()
                    .and_then(|g| g.vram_fraction())
                    .is_some_and(|f| f >= VRAM_FULL_FRACTION)
            }),
        ),
        (
            "diag.cause.throttle",
            Box::new(|p: &SamplePoint| {
                p.gpu
                    .as_ref()
                    .is_some_and(|g| g.throttle.iter().any(|r| THERMAL.contains(r)))
            }),
        ),
        (
            "diag.cause.cpu_full",
            Box::new(|p: &SamplePoint| p.avg_cpu_pct >= 90.0),
        ),
    ];
    let mut causes: Vec<Cause> = Vec::new();
    if a.spike_times.len() >= 3 {
        for (key, pred) in &checks {
            let (at_spikes, base) = a.coincidence(pred.as_ref());
            if at_spikes >= 0.5 && at_spikes >= base * 2.0 {
                causes.push(Cause { key, at_spikes });
            }
        }
    }
    causes.sort_by(|x, y| y.at_spikes.total_cmp(&x.at_spikes));

    let impact = if ratio < 0.35 {
        Impact::High
    } else {
        Impact::Moderate
    };
    let id = if causes.is_empty() {
        "stutter.unknown"
    } else {
        "stutter.found"
    };
    let mut d = draft(id, Category::Stutter, impact, 90)
        .arg("low", format!("{:.0}", s.low_1pct_fps))
        .arg("avg", format!("{:.0}", s.avg_fps))
        .arg("spikes", s.spikes)
        .advice()
        .evidence(
            Text::new("diag.ev.low_vs_avg"),
            Text::new("diag.ev.fps_pair")
                .arg("low", format!("{:.0}", s.low_1pct_fps))
                .arg("avg", format!("{:.0}", s.avg_fps)),
        )
        .evidence(
            Text::new("diag.ev.max_frametime"),
            Text::new("value.ms").arg("v", format!("{:.1}", s.max_frametime_ms)),
        );
    // Сам факт фризів виміряний (90 %); впевненість у причині — за частотою збігів.
    if let Some(top) = causes.first() {
        d = d.confidence(conf((40.0 + 40.0 * top.at_spikes).min(85.0)));
    }
    for c in &causes {
        d = d.evidence(Text::new(c.key), pct(c.at_spikes));
    }
    Some(d.build())
}

/// Поодинокі ривки: самі по собі ще не «мікрофризи», але саме вони тягнуть 0.1% low вниз,
/// тож пояснюємо користувачу, звідки ця цифра, замість того щоб мовчати.
fn isolated_hitches(a: &Analysis<'_>) -> Option<Finding> {
    let s = &a.stats;
    if s.spikes == 0 || s.low_01pct_fps >= s.avg_fps * 0.5 {
        return None;
    }
    let first_s = a.spike_times.first().copied().unwrap_or_default() / 1000.0;
    Some(
        draft("stutter.isolated", Category::Stutter, Impact::Info, 90)
            .arg("spikes", s.spikes)
            .arg("max", format!("{:.0}", s.max_frametime_ms))
            .arg("first", format!("{first_s:.1}"))
            .evidence(
                Text::new("diag.ev.max_frametime"),
                Text::new("value.ms").arg("v", format!("{:.1}", s.max_frametime_ms)),
            )
            .evidence(
                Text::new("diag.ev.first_spike"),
                Text::new("value.seconds").arg("v", format!("{first_s:.1}")),
            )
            .build(),
    )
}

/// Фонові процеси з помітним CPU під час гри (середнє за весь запис).
pub fn background(a: &Analysis<'_>) -> Vec<Finding> {
    let n = a.samples().len();
    if n == 0 {
        return Vec::new();
    }
    let mut sums: HashMap<&str, f64> = HashMap::new();
    for s in a.samples() {
        for (name, v) in &s.background {
            *sums.entry(name).or_default() += f64::from(*v);
        }
    }
    let mut heavy: Vec<(&str, f64)> = sums
        .into_iter()
        .map(|(name, sum)| (name, sum / n as f64))
        .filter(|(_, avg)| *avg >= f64::from(BACKGROUND_CPU_PCT))
        .collect();
    heavy.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(y.0)));
    heavy
        .into_iter()
        .take(3)
        .map(|(name, avg)| {
            draft("background", Category::Background, Impact::Moderate, 75)
                .arg("name", name)
                .arg("cpu", format!("{avg:.1}"))
                .advice()
                .evidence(
                    Text::new("diag.ev.process_cpu"),
                    Text::new("value.pct").arg("v", format!("{avg:.1}")),
                )
                .build()
        })
        .collect()
}
