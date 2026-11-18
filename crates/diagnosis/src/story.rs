//! Розповідь простими словами: «Як іде гра / Чому були ривки / Що зробити».
//!
//! Складається з уже обчисленого результату (знахідки, вузьке місце, статистика), тож не може
//! розійтися зі знахідками нижче. Нових порогів тут немає: оцінка плавності — з правила
//! мікрофризів, «чи це проблема» — із впливу знахідок. Причина ривка — збіг у часі, а не
//! доведений факт, тому тексти кажуть «найімовірніше» і «збіглося з».

use crate::rules::{self, Analysis, CauseKind};
use crate::{Bottleneck, Diagnosis, Verdict};
use optimax_core::{Impact, Text};
use serde::{Deserialize, Serialize};

/// Скільки причин показувати в «Чому були ривки».
const MAX_CAUSES: usize = 3;

/// Впевненість межового вердикту «відеокарта» (CLAUDE.md розд. 9) — тоді кажемо «схоже».
const BORDERLINE_CONFIDENCE: u8 = 60;

/// Наскільки плавно йшла гра (з правила мікрофризів).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Smoothness {
    /// Ривків немає.
    Smooth,
    /// Кілька окремих ривків, без постійних мікрофризів.
    Isolated,
    /// Помітні мікрофризи.
    Noticeable,
    /// Сильні мікрофризи.
    Severe,
}

/// Чи це проблема і чи є що робити.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StoryOutcome {
    /// Усе гаразд: це межа заліза.
    Fine,
    /// Є що виправити.
    Fixable,
    /// Ривки є, а причину не знайдено.
    Unexplained,
    /// Замало даних для висновку.
    Insufficient,
}

/// Можлива причина ривків у розповіді.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StoryCause {
    /// Код ознаки (`CauseKind::code`): зв'язок зі шкалою й радаром.
    pub code: String,
    /// Готове речення.
    pub text: Text,
    /// Головна (збіглася щонайменше з половиною ривків) чи часткова.
    pub likely: bool,
    /// У скількох ривках збіглася.
    pub hits: u32,
    /// З-поміж скількох ривків.
    pub of: u32,
}

/// Розповідь простими словами над результатом діагностики.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Story {
    /// Оцінка плавності.
    pub smoothness: Smoothness,
    /// Чи це проблема.
    pub outcome: StoryOutcome,
    /// «Як іде гра»: речення з цифрами.
    pub how: Vec<Text>,
    /// «Чому були ривки»: до трьох причин; порожньо — ривків немає або причину не знайдено.
    pub why: Vec<StoryCause>,
    /// Ривки є, а причину не знайдено.
    pub why_unknown: bool,
    /// Головна дія — індекс знахідки в `Diagnosis.verdicts`.
    pub action: Option<u32>,
    /// «Інші варіанти» — індекси знахідок у `Diagnosis.verdicts`.
    pub others: Vec<u32>,
}

/// Розповідь, коли даних замало для висновку.
pub fn insufficient_story() -> Story {
    Story {
        smoothness: Smoothness::Smooth,
        outcome: StoryOutcome::Insufficient,
        how: vec![Text::new("diag.story.how.insufficient")],
        why: Vec::new(),
        why_unknown: false,
        action: None,
        others: Vec::new(),
    }
}

/// Оцінка плавності з результату правила мікрофризів (`stutter.*`).
fn smoothness(verdicts: &[Verdict]) -> Smoothness {
    let stutter = verdicts
        .iter()
        .map(|v| &v.finding)
        .find(|f| f.id.starts_with("stutter."));
    match stutter {
        None => Smoothness::Smooth,
        Some(f) if f.id == "stutter.isolated" => Smoothness::Isolated,
        Some(f) if f.impact >= Impact::High => Smoothness::Severe,
        Some(_) => Smoothness::Noticeable,
    }
}

/// «Чому були ривки»: головні причини, далі часткові — та сама класифікація, що в `stutter`.
fn why(a: &Analysis<'_>) -> Vec<StoryCause> {
    let c = rules::stutter_causes(a);
    let of = u32::try_from(c.spikes).unwrap_or(u32::MAX);
    let main = c.main.iter().map(|x| (x, true));
    let partial = c.partial.iter().map(|x| (x, false));
    main.chain(partial)
        .take(MAX_CAUSES)
        .map(|(cause, likely)| {
            let hits = ((cause.at_spikes * f64::from(of)).round() as u32).clamp(1, of.max(1));
            let kind = if likely { "likely" } else { "partial" };
            let text = match (cause.kind, c.driver_name.as_deref()) {
                (CauseKind::Dpc, Some(name)) => {
                    Text::new(format!("diag.story.why.{kind}.dpc_driver")).arg("name", name)
                }
                _ => Text::new(format!("diag.story.why.{kind}.{}", cause.kind.code())),
            }
            .arg("hits", hits)
            .arg("of", of);
            StoryCause {
                code: cause.kind.code().to_string(),
                text,
                likely,
                hits,
                of,
            }
        })
        .collect()
}

/// «Як іде гра»: FPS, плавність, що обмежує.
fn how(
    a: &Analysis<'_>,
    bottleneck: Bottleneck,
    primary_conf: u8,
    smooth: Smoothness,
) -> Vec<Text> {
    let s = &a.stats;
    let avg = format!("{:.0}", s.avg_fps);
    let low = format!("{:.0}", s.low_1pct_fps);
    let fps = match (bottleneck, a.display_hz.filter(|hz| *hz > 0)) {
        (Bottleneck::FrameCap, _) => Text::new("diag.story.how.cap").arg("avg", &avg),
        (_, Some(hz)) => Text::new("diag.story.how.fps_hz")
            .arg("avg", &avg)
            .arg("hz", hz),
        (_, None) => Text::new("diag.story.how.fps").arg("avg", &avg),
    };
    let first_s = a.spike_times.first().copied().unwrap_or_default() / 1000.0;
    let smooth_text = match smooth {
        // Піки на шкалі є, але правило мікрофризів їх ривками не вважає — не кажемо «не було».
        Smoothness::Smooth if s.spikes > 0 => {
            Text::new("diag.story.how.smooth_minor").arg("spikes", s.spikes)
        }
        Smoothness::Smooth => Text::new("diag.story.how.smooth"),
        Smoothness::Isolated => Text::new("diag.story.how.isolated")
            .arg("spikes", s.spikes)
            .arg("first", format!("{first_s:.1}")),
        Smoothness::Noticeable => Text::new("diag.story.how.noticeable")
            .arg("low", &low)
            .arg("avg", &avg)
            .arg("spikes", s.spikes),
        Smoothness::Severe => Text::new("diag.story.how.severe")
            .arg("low", &low)
            .arg("avg", &avg)
            .arg("spikes", s.spikes)
            .arg("max", format!("{:.0}", s.max_frametime_ms)),
    };
    let mut out = vec![fps, smooth_text];
    let reason = match bottleneck {
        Bottleneck::Gpu if primary_conf <= BORDERLINE_CONFIDENCE => {
            Some("diag.story.how.gpu_borderline")
        }
        Bottleneck::Gpu => Some("diag.story.how.gpu"),
        Bottleneck::Cpu => Some("diag.story.how.cpu"),
        Bottleneck::FrameCap => Some("diag.story.how.cap_reason"),
        Bottleneck::Unclear => Some("diag.story.how.unclear"),
        Bottleneck::InsufficientData => None,
    };
    out.extend(reason.map(Text::new));
    out
}

/// Складає розповідь. `verdicts[0]` — підсумок; дії беруться з решти знахідок із порадою.
pub(crate) fn compose(
    a: &Analysis<'_>,
    bottleneck: Bottleneck,
    primary_conf: u8,
    verdicts: &[Verdict],
) -> Story {
    if bottleneck == Bottleneck::InsufficientData {
        return insufficient_story();
    }
    let smooth = smoothness(verdicts);
    let why = if matches!(smooth, Smoothness::Noticeable | Smoothness::Severe) {
        why(a)
    } else {
        Vec::new()
    };
    let why_unknown =
        matches!(smooth, Smoothness::Noticeable | Smoothness::Severe) && why.is_empty();

    let (outcome, action, others) = actions(verdicts, why_unknown);

    Story {
        smoothness: smooth,
        outcome,
        how: how(a, bottleneck, primary_conf, smooth),
        why,
        why_unknown,
        action,
        others,
    }
}

/// «Чи це проблема» і «Що зробити»: індекси знахідок із порадою в `verdicts` (крім підсумку).
fn actions(verdicts: &[Verdict], why_unknown: bool) -> (StoryOutcome, Option<u32>, Vec<u32>) {
    let candidates: Vec<u32> = verdicts
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, v)| v.finding.advice.is_some())
        .filter_map(|(i, _)| u32::try_from(i).ok())
        .collect();
    let finding = |i: u32| &verdicts[i as usize].finding;
    let fixable = candidates
        .iter()
        .copied()
        .find(|&i| finding(i).impact >= Impact::Moderate && finding(i).id != "stutter.unknown");
    let unknown = candidates
        .iter()
        .copied()
        .find(|&i| finding(i).id == "stutter.unknown");
    let (outcome, action) = match (fixable, unknown) {
        (Some(i), _) => (StoryOutcome::Fixable, Some(i)),
        (None, Some(i)) if why_unknown => (StoryOutcome::Unexplained, Some(i)),
        _ => (StoryOutcome::Fine, None),
    };
    let others = candidates
        .into_iter()
        .filter(|i| Some(*i) != action)
        .collect();
    (outcome, action, others)
}

/// Оновлює розповідь після зміни знахідок (генерація кадрів, тест роздільності): дії — заново за
/// поточним списком, щоб індекси вказували на ті самі знахідки. Тест, що не підтвердив
/// відеокарту, прибирає з розповіді «відеокарта на межі».
pub fn refresh(d: &mut Diagnosis) {
    let Some(story) = d.story.as_mut() else {
        return;
    };
    if story.outcome == StoryOutcome::Insufficient {
        return;
    }
    let (outcome, action, others) = actions(&d.verdicts, story.why_unknown);
    story.outcome = outcome;
    story.action = action;
    story.others = others;
    if d.verdicts.iter().any(|v| v.finding.id == "res.no_help") {
        for t in &mut story.how {
            if t.key == "diag.story.how.gpu" || t.key == "diag.story.how.gpu_borderline" {
                *t = Text::new("diag.story.how.gpu_unconfirmed");
            }
        }
    }
}
