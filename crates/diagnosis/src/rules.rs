//! Правила вердиктів. Кожне — чиста функція над [`Analysis`], повертає знахідку з
//! впевненістю й доказами або `None`, якщо ознак недостатньо. Пороги — з CLAUDE.md, розд. 9.

use crate::timeline::{SamplePoint, ShaderWrite, Timeline};
use optimax_core::{Category, Finding, FindingBuilder, Impact, Text};
use optimax_frametime::{find_spikes, FrameStats, PresentMode};
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
/// Рушій сканування Microsoft Defender.
pub const DEFENDER_PROCESS: &str = "MsMpEng.exe";
/// Сканування Defender помітне від цієї частки всього CPU (навіть один потік на багатоядерному
/// процесорі — це кілька відсотків).
pub const DEFENDER_CPU_PCT: f32 = 2.0;
/// Мінімальний сплеск жорстких page faults, сторінок/с (≈ 2 МБ/с при сторінці 4 КБ).
pub const HARD_FAULT_MIN: f64 = 500.0;
/// Пам'ять майже заповнена нижче цієї частки вільної RAM. Без цього сплеск page faults —
/// це звичайне читання файлів гри (причина «диск»), а не нестача пам'яті.
pub const LOW_RAM_FRACTION: f64 = 0.15;
/// Процес називаємо винуватцем диска біля ривка, лише якщо на нього припадає щонайменше
/// така частка загальної активності диска.
pub const IO_NAME_MIN_SHARE: f64 = 0.25;
/// Мінімальний сплеск часу на DPC і переривання на одному ядрі, %. Навіть без навантаження
/// він коливається в межах кількох відсотків, тож нижче цього збіг із ривком — шум.
pub const DPC_MIN_PCT: f64 = 5.0;
/// Ознака пояснює частину ривків, якщо трапляється біля такої частки з них.
pub const PARTIAL_MIN_SHARE: f64 = 0.2;
/// Вікно до ривка для записів у кеш шейдерів.
pub const SHADER_BEFORE_MS: f64 = 200.0;
/// Вікно після ривка: драйвер пише файл кешу під час компіляції або невдовзі після неї.
pub const SHADER_AFTER_MS: f64 = 1500.0;

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
    /// Піки: (час, мс; тривалість кадру, мс; локальна медіана, мс).
    pub spikes: Vec<(f64, f64, f64)>,
    /// Частота монітора, Гц (якщо відома).
    pub display_hz: Option<u32>,
}

impl<'a> Analysis<'a> {
    /// Рахує статистику й піки. `None`, якщо кадрів немає.
    pub fn new(timeline: &'a Timeline, display_hz: Option<u32>) -> Option<Self> {
        let ft: Vec<f64> = timeline.frames.iter().map(|f| f.frametime_ms).collect();
        let stats = optimax_frametime::frame_stats(&ft)?;
        let spikes: Vec<(f64, f64, f64)> = find_spikes(&ft)
            .iter()
            .map(|s| (timeline.frames[s.index].t_ms, s.frametime_ms, s.baseline_ms))
            .collect();
        let spike_times = spikes.iter().map(|s| s.0).collect();
        Some(Self {
            timeline,
            stats,
            spike_times,
            spikes,
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

/// Кадри гри здебільшого показуються через композицію DWM (`Composed: …` у PresentMon), а не
/// напряму (Independent Flip, апаратний оверлей, ексклюзивний повний екран).
///
/// Це прямий факт із PresentMon, тож впевненість висока. Скільки затримки додає композиція
/// саме в цій грі, не вимірюється: порада — перевірити й порівняти новим записом.
/// Джерело класифікації: таблиця PresentMode у `README-ConsoleApplication.md` PresentMon 2.6.0.
pub fn composed_present(a: &Analysis<'_>) -> Option<Finding> {
    let frames = &a.timeline.frames;
    let mut counts: HashMap<PresentMode, usize> = HashMap::new();
    let mut known = 0usize;
    for f in frames {
        if f.present_mode.is_composed().is_some() {
            known += 1;
            *counts.entry(f.present_mode).or_default() += 1;
        }
    }
    // Колонки немає або режим не розпізнано в більшості кадрів — висновків не робимо.
    if known == 0 || known * 2 < frames.len() {
        return None;
    }
    let composed: Vec<_> = frames
        .iter()
        .filter(|f| f.present_mode.is_composed() == Some(true))
        .collect();
    let share = composed.len() as f64 / known as f64;
    if share < MOSTLY {
        return None;
    }
    let top = counts
        .iter()
        .filter(|(m, _)| m.is_composed() == Some(true))
        .max_by(|x, y| x.1.cmp(y.1).then(y.0.as_str().cmp(x.0.as_str())))
        .map_or(PresentMode::ComposedFlip, |(m, _)| *m);
    let mut d = draft(
        "present.composed",
        Category::Display,
        Impact::Info,
        conf(70.0 + 25.0 * share),
    )
    .arg("share", format!("{:.0}", share * 100.0))
    .arg("mode", top.as_str())
    .advice()
    .evidence(Text::new("diag.ev.composed_share"), pct(share))
    .evidence(
        Text::new("diag.ev.present_mode"),
        Text::new("value.raw").arg("v", top.as_str()),
    );
    if let Some(si) = median(
        composed
            .iter()
            .filter_map(|f| f.sync_interval.map(f64::from))
            .collect(),
    ) {
        d = d.evidence(
            Text::new("diag.ev.sync_interval"),
            Text::new("value.raw").arg("v", format!("{si:.0}")),
        );
    }
    let tearing: Vec<bool> = composed.iter().filter_map(|f| f.allows_tearing).collect();
    if !tearing.is_empty() {
        let on = tearing.iter().filter(|t| **t).count() as f64 / tearing.len() as f64;
        d = d.evidence(Text::new("diag.ev.allows_tearing"), pct(on));
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
    let (at_spikes, usual) = a.coincidence(&active);
    let loss = clock_loss(a, &active);
    // Вплив — за виміряною втратою частоти; не виміряно — помірний, а не «найгірший».
    let impact = match loss {
        Some(l) if l.fraction >= CLOCK_LOSS_HIGH => Impact::High,
        Some(l) if l.fraction < CLOCK_LOSS_MODERATE => Impact::Info,
        _ => Impact::Moderate,
    };
    let mut d = draft("throttling", Category::Throttling, impact, 80)
        .arg("share", format!("{:.0}", share * 100.0))
        .advice()
        .evidence(Text::new("diag.ev.throttle_share"), pct(share))
        .evidence(
            Text::new("diag.ev.spikes_with_factor"),
            Text::new("diag.ev.spikes_vs_usual")
                .arg("pct", format!("{:.0}", at_spikes * 100.0))
                .arg("usual", format!("{:.0}", usual * 100.0)),
        );
    d = match loss {
        Some(l) => d.evidence(
            Text::new("diag.ev.throttle_clock"),
            Text::new("diag.ev.throttle_clock_value")
                .arg("on", l.throttled_mhz)
                .arg("off", l.free_mhz)
                .arg("pct", format!("{:.0}", l.fraction * 100.0)),
        ),
        None => d.evidence(
            Text::new("diag.ev.throttle_clock"),
            Text::new("diag.ev.throttle_clock_unknown"),
        ),
    };
    if let Some(t) = max_temp {
        d = d.evidence(
            Text::new("diag.ev.gpu_max_temp"),
            Text::new("value.celsius").arg("v", t),
        );
    }
    for r in reasons {
        d = d.evidence(Text::new("diag.ev.reason"), Text::new(r));
    }
    // Оберти під час тротлінгу: низькі при високій температурі — ручні оберти або несправний
    // вентилятор, а не «слабке охолодження».
    let fan = median(
        a.samples()
            .iter()
            .filter(|s| active(s))
            .filter_map(|s| s.gpu.as_ref()?.fan_pct.map(f64::from))
            .collect(),
    );
    if let Some(f) = fan {
        d = d.evidence(
            Text::new("diag.ev.gpu_fan"),
            Text::new("value.pct").arg("v", format!("{f:.0}")),
        );
        if f <= FAN_LOW_PCT && max_temp.is_some_and(|t| t >= FAN_HOT_C) {
            d = d.evidence(
                Text::new("diag.ev.what_to_do"),
                Text::new("diag.advice.fan_low")
                    .arg("fan", format!("{f:.0}"))
                    .arg("temp", max_temp.unwrap_or_default()),
            );
        }
    }
    // Ліміт живлення понад заводський (розгін): більше тепла, ніж розраховане охолодження.
    let limits = a.samples().iter().rev().find_map(|s| {
        let g = s.gpu.as_ref()?;
        Some((g.power_limit_w?, g.default_power_limit_w?))
    });
    if let Some((limit, default)) = limits.filter(|(l, d)| *d > 0.0 && *l > *d * POWER_RAISED) {
        d = d
            .evidence(
                Text::new("diag.ev.power_limit"),
                Text::new("diag.ev.power_limit_value")
                    .arg("w", format!("{limit:.0}"))
                    .arg("pct", format!("{:.0}", limit / default * 100.0))
                    .arg("default", format!("{default:.0}")),
            )
            .evidence(
                Text::new("diag.ev.what_to_do"),
                Text::new("diag.advice.power_raised"),
            );
    }
    Some(d.build())
}

/// Втрата частоти GPU через тротлінг: від цієї частки вплив високий.
pub const CLOCK_LOSS_HIGH: f64 = 0.08;
/// Нижче цієї частки частоту знижено ледь помітно.
pub const CLOCK_LOSS_MODERATE: f64 = 0.03;
/// Мінімум знімків під навантаженням у кожній групі (з тротлінгом і без) для порівняння.
const CLOCK_GROUP_MIN: usize = 5;
/// Оберти вентиляторів вважаються низькими до цього значення…
pub const FAN_LOW_PCT: f64 = 50.0;
/// …якщо при цьому GPU гріється щонайменше до цієї температури.
pub const FAN_HOT_C: u32 = 80;
/// Діючий ліміт живлення вважається піднятим понад заводський від цього множника.
const POWER_RAISED: f32 = 1.02;

/// Виміряна втрата частоти.
#[derive(Debug, Clone, Copy)]
struct ClockLoss {
    throttled_mhz: u32,
    free_mhz: u32,
    fraction: f64,
}

/// Медіанна частота GPU під навантаженням з тротлінгом і без. `None` — замало знімків
/// у якійсь групі (напр. тротлінг майже весь час), тоді втрату чесно не оцінюємо.
fn clock_loss(a: &Analysis<'_>, active: &dyn Fn(&SamplePoint) -> bool) -> Option<ClockLoss> {
    let (mut on, mut off) = (Vec::new(), Vec::new());
    for s in a.samples() {
        let Some(g) = s.gpu.as_ref() else { continue };
        let (Some(clock), true) = (g.clock_mhz, g.util.is_some_and(|u| u >= 90)) else {
            continue;
        };
        if active(s) {
            on.push(f64::from(clock));
        } else {
            off.push(f64::from(clock));
        }
    }
    if on.len() < CLOCK_GROUP_MIN || off.len() < CLOCK_GROUP_MIN {
        return None;
    }
    let (throttled, free) = (median(on)?, median(off)?);
    (free > 0.0).then(|| ClockLoss {
        throttled_mhz: throttled.round() as u32,
        free_mhz: free.round() as u32,
        fraction: ((free - throttled) / free).max(0.0),
    })
}

/// Можлива причина ривка frametime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CauseKind {
    /// Сканування Microsoft Defender.
    Defender,
    /// Нестача RAM: підкачка при майже заповненій пам'яті.
    Memory,
    /// Компіляція шейдерів: драйвер записує кеш шейдерів.
    Shader,
    /// Сплеск активності диска.
    Disk,
    /// Драйвери надовго займають ядро CPU (DPC і переривання).
    Dpc,
    /// Сплеск стороннього процесу.
    Background,
    /// Заповнена VRAM.
    Vram,
    /// Тепловий тротлінг GPU.
    Throttle,
    /// Повне завантаження CPU.
    CpuFull,
}

impl CauseKind {
    /// Усі причини в порядку показу (конкретніші — раніше).
    pub const ALL: [CauseKind; 9] = [
        Self::Defender,
        Self::Memory,
        Self::Shader,
        Self::Disk,
        Self::Dpc,
        Self::Background,
        Self::Vram,
        Self::Throttle,
        Self::CpuFull,
    ];

    /// Ключ тексту «збіг піків з …» (доказ у вердикті мікрофризів).
    pub fn key(self) -> &'static str {
        match self {
            Self::Defender => "diag.cause.defender",
            Self::Memory => "diag.cause.memory",
            Self::Shader => "diag.cause.shader",
            Self::Disk => "diag.cause.disk",
            Self::Dpc => "diag.cause.dpc",
            Self::Background => "diag.cause.background",
            Self::Vram => "diag.cause.vram",
            Self::Throttle => "diag.cause.throttle",
            Self::CpuFull => "diag.cause.cpu_full",
        }
    }

    /// Код ознаки (зв'язує зведення на радарі з ривками).
    pub fn code(self) -> &'static str {
        self.name_key().rsplit('.').next().unwrap_or_default()
    }

    /// Ключ короткої назви (зведення ривків на радарі).
    pub fn name_key(self) -> &'static str {
        match self {
            Self::Defender => "diag.cause_name.defender",
            Self::Memory => "diag.cause_name.memory",
            Self::Shader => "diag.cause_name.shader",
            Self::Disk => "diag.cause_name.disk",
            Self::Dpc => "diag.cause_name.dpc",
            Self::Background => "diag.cause_name.background",
            Self::Vram => "diag.cause_name.vram",
            Self::Throttle => "diag.cause_name.throttle",
            Self::CpuFull => "diag.cause_name.cpu_full",
        }
    }

    /// Ключ короткого опису біля конкретного ривка на графіку.
    fn spike_key(self) -> &'static str {
        match self {
            Self::Defender => "diag.spike.defender",
            Self::Memory => "diag.spike.memory",
            Self::Shader => "diag.spike.shader",
            Self::Disk => "diag.spike.disk",
            Self::Dpc => "diag.spike.dpc",
            Self::Background => "diag.spike.background",
            Self::Vram => "diag.spike.vram",
            Self::Throttle => "diag.spike.throttle",
            Self::CpuFull => "diag.spike.cpu_full",
        }
    }

    /// Порада для причини (окремим рядком доказів у вердикті мікрофризів).
    fn advice(self) -> Option<&'static str> {
        match self {
            Self::Defender => Some("diag.advice.defender"),
            Self::Memory => Some("diag.advice.memory"),
            Self::Shader => Some("diag.advice.shader"),
            Self::Dpc => Some("diag.advice.dpc"),
            _ => None,
        }
    }
}

fn is_defender(name: &str) -> bool {
    name.eq_ignore_ascii_case(DEFENDER_PROCESS)
}

/// Записи в кеш шейдерів у вікні навколо `t`.
fn shader_writes_near(tl: &Timeline, t: f64) -> &[ShaderWrite] {
    let w = &tl.shader_writes;
    let lo = w.partition_point(|x| x.t_ms < t - SHADER_BEFORE_MS);
    let hi = w.partition_point(|x| x.t_ms <= t + SHADER_AFTER_MS);
    &w[lo..hi.max(lo)]
}

/// Пороги причин, обчислені за весь запис («сплеск» = помітно вище за звичайне).
pub struct CauseContext {
    disk_limit: f64,
    fault_limit: f64,
    dpc_limit: f64,
    medians: HashMap<String, f32>,
}

impl CauseContext {
    /// Рахує пороги для запису.
    pub fn new(a: &Analysis<'_>) -> Self {
        let fault_median = median(
            a.samples()
                .iter()
                .filter_map(|s| s.hard_faults_per_sec)
                .collect(),
        )
        .unwrap_or(0.0);
        let dpc_median =
            median(a.samples().iter().filter_map(|s| s.dpc_pct).collect()).unwrap_or(0.0);
        Self {
            disk_limit: disk_threshold(a),
            fault_limit: (fault_median * 3.0).max(HARD_FAULT_MIN),
            dpc_limit: (dpc_median * 3.0).max(DPC_MIN_PCT),
            medians: background_medians(a),
        }
    }

    fn burst(&self, name: &str, v: f32) -> bool {
        v >= self.medians.get(name).copied().unwrap_or(0.0) * 3.0
    }

    /// Ознака в одному знімку. Кеш шейдерів знімками не описується — див. [`Self::hit_near`].
    fn sample_hit(&self, kind: CauseKind, p: &SamplePoint) -> bool {
        match kind {
            CauseKind::Defender => p
                .background
                .iter()
                .any(|(n, v)| is_defender(n) && *v >= DEFENDER_CPU_PCT && self.burst(n, *v)),
            CauseKind::Memory => {
                p.hard_faults_per_sec.is_some_and(|f| f >= self.fault_limit)
                    && p.mem_available_fraction
                        .is_some_and(|m| m < LOW_RAM_FRACTION)
            }
            CauseKind::Shader => false,
            CauseKind::Disk => p.disk_bytes_per_sec as f64 >= self.disk_limit,
            CauseKind::Dpc => p.dpc_pct.is_some_and(|v| v >= self.dpc_limit),
            CauseKind::Background => p
                .background
                .iter()
                .any(|(n, v)| !is_defender(n) && *v >= BACKGROUND_CPU_PCT && self.burst(n, *v)),
            CauseKind::Vram => p
                .gpu
                .as_ref()
                .and_then(|g| g.vram_fraction())
                .is_some_and(|f| f >= VRAM_FULL_FRACTION),
            CauseKind::Throttle => p
                .gpu
                .as_ref()
                .is_some_and(|g| g.throttle.iter().any(|r| THERMAL.contains(r))),
            CauseKind::CpuFull => p.avg_cpu_pct >= 90.0,
        }
    }

    /// Чи була ознака поруч із моментом `t` (мс від старту запису).
    pub fn hit_near(&self, a: &Analysis<'_>, kind: CauseKind, t: f64) -> bool {
        match kind {
            CauseKind::Shader => !shader_writes_near(a.timeline, t).is_empty(),
            _ => a
                .timeline
                .samples_near(t, WINDOW_MS)
                .iter()
                .any(|p| self.sample_hit(kind, p)),
        }
    }

    /// Частка піків з ознакою поруч і «фонова» частка (те саме навколо кожного знімка).
    pub(crate) fn coincidence(&self, a: &Analysis<'_>, kind: CauseKind) -> (f64, f64) {
        let share = |times: &mut dyn Iterator<Item = f64>| {
            let (mut hit, mut all) = (0usize, 0usize);
            for t in times {
                all += 1;
                if self.hit_near(a, kind, t) {
                    hit += 1;
                }
            }
            if all == 0 {
                0.0
            } else {
                hit as f64 / all as f64
            }
        };
        (
            share(&mut a.spike_times.iter().copied()),
            share(&mut a.samples().iter().map(|s| s.t_ms)),
        )
    }

    /// Що саме було біля ривка в момент `t` — короткий текст для позначки на графіку.
    pub fn detail(&self, a: &Analysis<'_>, kind: CauseKind, t: f64) -> Text {
        let near = a.timeline.samples_near(t, WINDOW_MS);
        let max_of = |f: &dyn Fn(&SamplePoint) -> Option<f64>| {
            near.iter().filter_map(f).fold(0.0_f64, f64::max)
        };
        let text = Text::new(kind.spike_key());
        match kind {
            CauseKind::Defender => text.arg(
                "cpu",
                format!(
                    "{:.0}",
                    max_of(&|p| p
                        .background
                        .iter()
                        .find(|(n, _)| is_defender(n))
                        .map(|(_, v)| f64::from(*v)))
                ),
            ),
            CauseKind::Memory => {
                let free = near
                    .iter()
                    .filter_map(|p| p.mem_available_fraction)
                    .fold(1.0_f64, f64::min);
                text.arg(
                    "faults",
                    format!("{:.0}", max_of(&|p| p.hard_faults_per_sec)),
                )
                .arg("free", format!("{:.0}", free * 100.0))
            }
            CauseKind::Shader => text.arg("files", shader_writes_near(a.timeline, t).len()),
            CauseKind::Disk => {
                let mb = |b: f64| format!("{:.0}", b / (1024.0 * 1024.0));
                let total_bytes = max_of(&|p| Some(p.disk_bytes_per_sec as f64));
                let total = mb(total_bytes);
                // Хто найбільше читав/писав поруч — щоб «диск» не був знеособленим. Процеси
                // міряються рідше за диск, тож короткий сплеск може не потрапити в їхній замір:
                // називаємо процес, лише якщо на нього припадає помітна частка, інакше ім'я
                // було б хибним.
                let who = near
                    .iter()
                    .filter_map(|p| p.io_top.as_ref())
                    .max_by_key(|io| io.bytes_per_sec)
                    .filter(|io| io.bytes_per_sec as f64 >= total_bytes * IO_NAME_MIN_SHARE);
                match who {
                    Some(io) => Text::new("diag.spike.disk_by")
                        .arg("mbps", total)
                        .arg("name", io.name.as_str())
                        .arg("pmbps", mb(io.bytes_per_sec as f64)),
                    None => text.arg("mbps", total),
                }
            }
            CauseKind::Dpc => text.arg("pct", format!("{:.1}", max_of(&|p| p.dpc_pct))),
            CauseKind::Background => {
                let top = near
                    .iter()
                    .flat_map(|p| p.background.iter())
                    .filter(|(n, v)| !is_defender(n) && self.burst(n, **v))
                    .max_by(|x, y| x.1.total_cmp(y.1));
                match top {
                    Some((n, v)) => text.arg("name", n.as_str()).arg("cpu", format!("{v:.0}")),
                    None => text.arg("name", "—").arg("cpu", "0"),
                }
            }
            CauseKind::Vram => text.arg(
                "pct",
                format!(
                    "{:.0}",
                    max_of(&|p| p.gpu.as_ref().and_then(|g| g.vram_fraction())) * 100.0
                ),
            ),
            CauseKind::Throttle => text,
            CauseKind::CpuFull => text.arg(
                "pct",
                format!("{:.0}", max_of(&|p| Some(f64::from(p.avg_cpu_pct)))),
            ),
        }
    }
}

/// Причина, що стабільно збігається з піками.
struct Cause {
    kind: CauseKind,
    at_spikes: f64,
    /// Частка звичайних моментів з цією ознакою.
    usual: f64,
}

/// Ознака біля піків вважається причиною, лише якщо там вона трапляється щонайменше
/// в стільки разів частіше, ніж у звичайні моменти.
pub const ABOVE_USUAL: f64 = 2.0;

/// Причини за частотою збігу з піками: (головні, часткові, тло), кожні — від найчастішої.
/// Головна — щонайменше в половині піків і вдвічі частіше, ніж у звичайні моменти.
/// Часткова — не дотягує до головної, але трапляється біля [`PARTIAL_MIN_SHARE`] піків і теж
/// вдвічі частіше за звичайне: пояснює частину ривків, коли вони мають різні причини.
/// Тло — біля помітної частки піків, але й у звичайні моменти не рідше: сама по собі ознака
/// ривків не пояснює (напр. гра постійно підвантажує світ з диска).
fn correlated_causes(a: &Analysis<'_>, ctx: &CauseContext) -> (Vec<Cause>, Vec<Cause>, Vec<Cause>) {
    if a.spike_times.len() < 3 {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let (mut main, mut partial, mut usual) = (Vec::new(), Vec::new(), Vec::new());
    for kind in CauseKind::ALL {
        let (at_spikes, base) = ctx.coincidence(a, kind);
        let c = Cause {
            kind,
            at_spikes,
            usual: base,
        };
        if at_spikes < PARTIAL_MIN_SHARE {
            continue;
        }
        if at_spikes < base * ABOVE_USUAL {
            usual.push(c);
        } else if at_spikes >= 0.5 {
            main.push(c);
        } else {
            partial.push(c);
        }
    }
    for v in [&mut main, &mut partial, &mut usual] {
        v.sort_by(|x, y| y.at_spikes.total_cmp(&x.at_spikes));
    }
    (main, partial, usual)
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

    let ctx = CauseContext::new(a);
    let (causes, partial, usual) = correlated_causes(a, &ctx);

    let impact = if ratio < 0.35 {
        Impact::High
    } else {
        Impact::Moderate
    };
    let id = if !causes.is_empty() {
        "stutter.found"
    } else if !partial.is_empty() {
        "stutter.partial"
    } else {
        "stutter.unknown"
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
    // Кеш шейдерів — непряма ознака (час зміни файлу), тож для неї стеля нижча.
    if let Some(top) = causes.first() {
        // DPC — теж непряма: лічильник не каже, чи заблокований драйвером саме потік гри.
        let ceiling = match top.kind {
            CauseKind::Shader => 65.0,
            CauseKind::Dpc => 70.0,
            _ => 85.0,
        };
        d = d.confidence(conf((40.0 + 40.0 * top.at_spikes).min(ceiling)));
    }
    for c in &causes {
        d = d.evidence(Text::new(c.kind.key()), pct(c.at_spikes));
    }
    for c in &partial {
        d = d.evidence(
            Text::new(c.kind.name_key()),
            Text::new("diag.ev.partial_share").arg("pct", format!("{:.0}", c.at_spikes * 100.0)),
        );
    }
    for c in &usual {
        d = d.evidence(
            Text::new(c.kind.name_key()),
            Text::new("diag.ev.as_usual")
                .arg("pct", format!("{:.0}", c.at_spikes * 100.0))
                .arg("usual", format!("{:.0}", c.usual * 100.0)),
        );
    }
    if causes.iter().any(|c| c.kind == CauseKind::Shader) {
        let w = &a.timeline.shader_writes;
        let bytes: u64 = w.iter().map(|x| x.bytes).sum();
        d = d.evidence(
            Text::new("diag.ev.shader_cache"),
            Text::new("diag.ev.shader_cache_value")
                .arg("files", w.len())
                .arg("mb", format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))),
        );
    }
    for advice in causes.iter().filter_map(|c| c.kind.advice()) {
        d = d.evidence(Text::new("diag.ev.what_to_do"), Text::new(advice));
    }
    Some(d.build())
}

/// Допуск відхилення кадру від кратного оновленню монітора (частка інтервалу оновлення).
const VSYNC_TOLERANCE: f64 = 0.12;
/// Кадри «кратні оновленню», якщо таких щонайменше стільки.
const VSYNC_QUANTIZED_MIN: f64 = 0.85;
/// Ривки V-Sync помітні, якщо пропущених оновлень (кадр на 2–3 інтервали) щонайменше стільки.
const VSYNC_MISSED_MIN: f64 = 0.05;

/// Ривки V-Sync без адаптивної синхронізації (G-Sync/FreeSync): кадр показується лише на
/// оновленні монітора, тож коли гра не встигає за частотою, кадр чекає наступного оновлення —
/// час кадру стрибає між 1 і 2 інтервалами (16.7 ↔ 33.3 мс на 60 Гц). З VRR кадри не
/// прив'язані до оновлень, і такого шаблону немає.
pub fn vsync_judder(a: &Analysis<'_>) -> Option<Finding> {
    let hz = f64::from(a.display_hz.filter(|hz| *hz > 0)?);
    let frames = &a.timeline.frames;
    let n = frames.len();
    // Явно вимкнений V-Sync (SyncInterval 0) — шаблон не про нього.
    let no_vsync = frames.iter().filter(|f| f.sync_interval == Some(0)).count();
    if n < 300 || no_vsync * 2 >= n {
        return None;
    }
    let interval = 1000.0 / hz;
    let mut at = [0usize; 3];
    for f in frames {
        let k = (f.frametime_ms / interval).round();
        if (1.0..=3.0).contains(&k)
            && (f.frametime_ms - k * interval).abs() <= interval * VSYNC_TOLERANCE
        {
            at[k as usize - 1] += 1;
        }
    }
    let share = |c: usize| c as f64 / n as f64;
    let quantized = share(at[0] + at[1] + at[2]);
    let missed = share(at[1] + at[2]);
    // Більшість кадрів на частоті монітора, частина — з пропущеним оновленням.
    if quantized < VSYNC_QUANTIZED_MIN || missed < VSYNC_MISSED_MIN || share(at[0]) < 0.3 {
        return None;
    }
    Some(
        draft("vsync.judder", Category::FrameCap, Impact::Moderate, 75)
            .arg("hz", format!("{hz:.0}"))
            .arg("ms1", format!("{interval:.1}"))
            .arg("ms2", format!("{:.1}", interval * 2.0))
            .advice()
            .evidence(Text::new("diag.ev.vsync_quantized"), pct(quantized))
            .evidence(Text::new("diag.ev.vsync_missed"), pct(missed))
            .evidence(
                Text::new("diag.ev.refresh"),
                Text::new("value.hz").arg("v", format!("{hz:.0}")),
            )
            .build(),
    )
}

/// Ривки ближче за це вважаються одним (кілька довгих кадрів поспіль).
const PERIODIC_MERGE_MS: f64 = 150.0;
/// Найкоротший період, який вважаємо «за таймером».
const PERIODIC_MIN_MS: f64 = 400.0;
/// Мінімум ривків для висновку про періодичність.
const PERIODIC_MIN_EVENTS: usize = 6;
/// Інтервал «такий самий», якщо відхиляється від медіанного не більше ніж на цю частку.
const PERIODIC_TOLERANCE: f64 = 0.15;
/// Частка інтервалів, що мають збігатися з медіанним.
const PERIODIC_MIN_SHARE: f64 = 0.7;

/// Ривки з рівним періодом (наприклад, кожні 2 с): так поводиться щось за таймером —
/// опитування пристроїв, утиліти моніторингу чи підсвітки, драйвер, автозбереження гри.
/// Сцена гри так рівно не повторюється.
pub fn periodic_spikes(a: &Analysis<'_>) -> Option<Finding> {
    let mut events: Vec<f64> = Vec::new();
    for &t in &a.spike_times {
        if events
            .last()
            .is_none_or(|last| t - last > PERIODIC_MERGE_MS)
        {
            events.push(t);
        }
    }
    if events.len() < PERIODIC_MIN_EVENTS {
        return None;
    }
    let intervals: Vec<f64> = events.windows(2).map(|w| w[1] - w[0]).collect();
    let period = median(intervals.clone())?;
    if period < PERIODIC_MIN_MS {
        return None;
    }
    let regular = intervals
        .iter()
        .filter(|i| (*i - period).abs() <= period * PERIODIC_TOLERANCE)
        .count() as f64
        / intervals.len() as f64;
    if regular < PERIODIC_MIN_SHARE {
        return None;
    }
    let seconds = format!("{:.1}", period / 1000.0);
    Some(
        draft("stutter.periodic", Category::Stutter, Impact::Info, 70)
            .arg("period", seconds.as_str())
            .arg("events", events.len())
            .advice()
            .evidence(
                Text::new("diag.ev.spike_period"),
                Text::new("value.seconds").arg("v", seconds.as_str()),
            )
            .evidence(Text::new("diag.ev.spike_period_regular"), pct(regular))
            .build(),
    )
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
            // Defender не «закривають»: порада інша — перенести сканування, а не вимикати захист.
            let id = if is_defender(name) {
                "background.defender"
            } else {
                "background"
            };
            draft(id, Category::Background, Impact::Moderate, 75)
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
