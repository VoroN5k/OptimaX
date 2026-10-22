//! «Чому лагає?» — визначення вузького місця за кадрами й метриками.
//!
//! Лише чисті функції над даними: жодних системних викликів, тож усе покривається тестами.
//! Кожен вердикт має рівень впевненості й докази; якщо даних мало, так і кажемо.
//! `expected_gain` заповнюється тільки після реального A/B-вимірювання (етап 3).

#![forbid(unsafe_code)]

pub mod rules;
pub mod timeline;

pub use timeline::{FramePoint, GpuPoint, SamplePoint, Timeline};

use optimax_core::{Category, Finding, FindingBuilder, Impact, Text};
use optimax_frametime::{FrameStats, Recording};
use rules::Analysis;
use serde::{Deserialize, Serialize};

/// Менше кадрів — висновків не робимо.
pub const MIN_FRAMES: usize = 200;
/// Менше знімків метрик — висновків не робимо.
pub const MIN_SAMPLES: usize = 10;
/// Мінімальна тривалість запису, мс.
pub const MIN_DURATION_MS: f64 = 10_000.0;

/// Виміряний ефект зміни (заповнюється A/B-тестом, не діагностикою).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MeasuredGain {
    /// Зміна середнього FPS, %.
    pub avg_fps_change_pct: f64,
    /// Зміна 1% low, %.
    pub low_1pct_change_pct: f64,
    /// Скільки прогонів «до» і «після».
    pub runs: u32,
}

/// Вердикт: знахідка + виміряний ефект виправлення (якщо вже міряли).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Verdict {
    /// Знахідка з категорією, впливом, впевненістю й доказами.
    pub finding: Finding,
    /// Лише після реального вимірювання.
    pub expected_gain: Option<MeasuredGain>,
}

impl From<Finding> for Verdict {
    fn from(finding: Finding) -> Self {
        Self {
            finding,
            expected_gain: None,
        }
    }
}

/// Головне обмеження продуктивності.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Bottleneck {
    /// Відеокарта.
    Gpu,
    /// Процесор.
    Cpu,
    /// Ліміт FPS / V-Sync.
    FrameCap,
    /// Однозначного обмеження не знайдено.
    Unclear,
    /// Замало даних для висновку.
    InsufficientData,
}

/// Результат діагностики.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Diagnosis {
    /// Ім'я процесу гри.
    pub game: String,
    /// Статистика кадрів.
    pub stats: Option<FrameStats>,
    /// Головне обмеження.
    pub bottleneck: Bottleneck,
    /// Вердикти: спершу підсумок, далі за впливом і впевненістю.
    pub verdicts: Vec<Verdict>,
}

/// Додатковий контекст.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Частота монітора, на якому гра (для розпізнавання V-Sync).
    pub display_hz: Option<u32>,
    /// Знахідки аудиту конфігурації; у звіт потрапляють ті, що вище інформаційного рівня.
    pub config_findings: Vec<Finding>,
}

fn summary(
    id: &'static str,
    confidence: u8,
    stats: Option<&FrameStats>,
    game: &str,
    seconds: f64,
) -> Finding {
    let mut d = FindingBuilder::new("diag", id, Category::Summary, Impact::Info, confidence)
        .arg("game", game)
        .arg("seconds", format!("{seconds:.0}"));
    if let Some(s) = stats {
        d = d
            .arg("avg", format!("{:.1}", s.avg_fps))
            .arg("low1", format!("{:.1}", s.low_1pct_fps))
            .arg("low01", format!("{:.1}", s.low_01pct_fps))
            .arg("frames", s.frames)
            .evidence(
                Text::new("diag.ev.avg_fps"),
                Text::new("value.fps").arg("v", format!("{:.1}", s.avg_fps)),
            )
            .evidence(
                Text::new("diag.ev.low1"),
                Text::new("value.fps").arg("v", format!("{:.1}", s.low_1pct_fps)),
            )
            .evidence(
                Text::new("diag.ev.low01"),
                Text::new("value.fps").arg("v", format!("{:.1}", s.low_01pct_fps)),
            )
            .evidence(
                Text::new("diag.ev.p99"),
                Text::new("value.ms").arg("v", format!("{:.2}", s.p99_frametime_ms)),
            );
    }
    d.build()
}

/// Діагностика за шкалою часу.
pub fn diagnose_timeline(tl: &Timeline, ctx: &Context) -> Diagnosis {
    let analysis = Analysis::new(tl, ctx.display_hz);
    let stats = analysis.as_ref().map(|a| a.stats.clone());
    let seconds = tl.duration_ms() / 1000.0;
    let enough = analysis.is_some()
        && tl.frames.len() >= MIN_FRAMES
        && tl.samples.len() >= MIN_SAMPLES
        && tl.duration_ms() >= MIN_DURATION_MS;
    let Some(a) = analysis.filter(|_| enough) else {
        return Diagnosis {
            game: tl.game_name.clone(),
            bottleneck: Bottleneck::InsufficientData,
            verdicts: vec![summary(
                "summary.insufficient",
                100,
                stats.as_ref(),
                &tl.game_name,
                seconds,
            )
            .into()],
            stats,
        };
    };

    let cap = rules::frame_cap(&a);
    // При стабільному ліміті FPS ядро «в стелі» — це зазвичай очікування, а не нестача CPU.
    let (gpu, cpu) = if cap.is_some() {
        (None, None)
    } else {
        (rules::gpu_bound(&a), rules::cpu_bound(&a))
    };

    let (bottleneck, primary_conf) = match (&cap, &gpu, &cpu) {
        (Some(c), _, _) => (Bottleneck::FrameCap, c.confidence.percent()),
        (None, Some(g), Some(c)) if c.confidence > g.confidence => {
            (Bottleneck::Cpu, c.confidence.percent())
        }
        (None, Some(g), _) => (Bottleneck::Gpu, g.confidence.percent()),
        (None, None, Some(c)) => (Bottleneck::Cpu, c.confidence.percent()),
        (None, None, None) => (Bottleneck::Unclear, 100),
    };
    let summary_id = match bottleneck {
        Bottleneck::Gpu => "summary.gpu",
        Bottleneck::Cpu => "summary.cpu",
        Bottleneck::FrameCap => "summary.cap",
        Bottleneck::Unclear | Bottleneck::InsufficientData => "summary.unclear",
    };

    let mut found: Vec<Finding> = [
        cap,
        gpu,
        cpu,
        rules::vram_pressure(&a),
        rules::throttling(&a),
        rules::stutter(&a),
    ]
    .into_iter()
    .flatten()
    .collect();
    found.extend(rules::background(&a));
    found.extend(
        ctx.config_findings
            .iter()
            .filter(|f| f.impact > Impact::Info)
            .cloned(),
    );
    found.sort_by(|x, y| {
        y.impact
            .cmp(&x.impact)
            .then(y.confidence.cmp(&x.confidence))
    });

    let mut verdicts: Vec<Verdict> = vec![summary(
        summary_id,
        primary_conf,
        Some(&a.stats),
        &tl.game_name,
        seconds,
    )
    .into()];
    verdicts.extend(found.into_iter().map(Verdict::from));
    Diagnosis {
        game: tl.game_name.clone(),
        stats: Some(a.stats),
        bottleneck,
        verdicts,
    }
}

/// Діагностика запису. Якщо кадрів гри немає — «недостатньо даних».
pub fn diagnose(rec: &Recording, ctx: &Context) -> Diagnosis {
    match Timeline::from_recording(rec) {
        Some(tl) => diagnose_timeline(&tl, ctx),
        None => Diagnosis {
            game: String::new(),
            stats: None,
            bottleneck: Bottleneck::InsufficientData,
            verdicts: vec![summary("summary.insufficient", 100, None, "", 0.0).into()],
        },
    }
}

#[cfg(test)]
mod tests;
