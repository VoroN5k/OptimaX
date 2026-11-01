//! Генерація кадрів (DLSS FG, FSR FG, AFMF): гра подає на показ кадри парами — справжній і
//! згенерований майже одночасно, — а рівномірно на екрані їх розставляє драйвер. Тому «час між
//! Present()» стає пилкою (≈0.5 мс, ≈17 мс, ≈0.5 мс, …): кожен справжній кадр виглядав би як
//! ривок відносно «звичайних» 0.5 мс, а 1% low — у рази нижчим, ніж бачить око.
//!
//! Тут генерацію розпізнаємо за цією пилкою й будуємо шкалу кадрів, яку бачить людина:
//! - якщо PresentMon дав час між змінами картинки на екрані (`MsBetweenDisplayChange`) майже
//!   для всіх кадрів і цей ряд рівний — беремо його;
//! - інакше зливаємо кожен короткий кадр із наступним справжнім — виходить ряд справжніх кадрів.

use crate::timeline::{FramePoint, Timeline};
use serde::{Deserialize, Serialize};

/// Кадр «короткий», якщо він коротший за цю частку від 75-го перцентиля.
pub const SHORT_FRACTION: f64 = 0.35;
/// Генерація кадрів — якщо коротких кадрів щонайменше стільки (для ×2 — близько половини).
pub const FG_MIN_SHARE: f64 = 0.3;
/// Час між змінами на екрані беремо, якщо він є щонайменше для такої частки кадрів.
pub const DISPLAY_MIN_COVERAGE: f64 = 0.8;

/// За яким рядом рахувалась діагностика при генерації кадрів.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum FrameGenSeries {
    /// Час між змінами картинки на екрані (те, що бачить око).
    Display,
    /// Справжні кадри (короткі згенеровані злито з наступним справжнім).
    Real,
}

/// Виявлена генерація кадрів.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FrameGenInfo {
    /// Кадрів на секунду, поданих на показ (справжні + згенеровані).
    pub presented_fps: f64,
    /// Справжніх кадрів на секунду (від них залежить затримка вводу).
    pub real_fps: f64,
    /// Множник генерації (≈2 для ×2).
    pub factor: f64,
    /// За яким рядом рахувалась решта діагностики.
    pub series: FrameGenSeries,
}

fn p75(v: &[f64]) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return 0.0;
    }
    s.sort_by(f64::total_cmp);
    s[(s.len() * 3 / 4).min(s.len() - 1)]
}

/// Частка «коротких» кадрів. `None` — замало кадрів для висновку.
pub fn short_share(ft: &[f64]) -> Option<f64> {
    if ft.len() < 50 {
        return None;
    }
    let limit = p75(ft) * SHORT_FRACTION;
    let short = ft.iter().filter(|&&x| x < limit).count();
    Some(short as f64 / ft.len() as f64)
}

/// Чи схожий ряд frametime на генерацію кадрів (пилка з коротких і звичайних кадрів).
pub fn looks_generated(ft: &[f64]) -> bool {
    short_share(ft).is_some_and(|s| s >= FG_MIN_SHARE)
}

/// Шкала кадрів, яку бачить людина, і відомості про генерацію. Без генерації — `None`
/// (діагностика йде за звичайною шкалою).
pub fn effective_timeline(tl: &Timeline) -> Option<(Timeline, FrameGenInfo)> {
    let ft: Vec<f64> = tl.frames.iter().map(|f| f.frametime_ms).collect();
    if !looks_generated(&ft) {
        return None;
    }
    let limit = p75(&ft) * SHORT_FRACTION;
    let duration_s = ft.iter().sum::<f64>() / 1000.0;
    let real = ft.iter().filter(|&&x| x >= limit).count();
    let presented_fps = tl.frames.len() as f64 / duration_s.max(f64::EPSILON);
    let real_fps = real as f64 / duration_s.max(f64::EPSILON);

    // 1. Час між змінами на екрані — якщо він є майже для всіх кадрів і сам не «пилка».
    let shown: Vec<FramePoint> = tl
        .frames
        .iter()
        .filter_map(|f| {
            let d = f.display_ms.filter(|d| *d > 0.0)?;
            Some(FramePoint {
                frametime_ms: d,
                ..*f
            })
        })
        .collect();
    let coverage = shown.len() as f64 / tl.frames.len() as f64;
    let shown_ft: Vec<f64> = shown.iter().map(|f| f.frametime_ms).collect();
    let (frames, series) = if coverage >= DISPLAY_MIN_COVERAGE && !looks_generated(&shown_ft) {
        (shown, FrameGenSeries::Display)
    } else {
        // 2. Справжні кадри: короткі (згенеровані) додаємо до наступного справжнього.
        let mut out = Vec::with_capacity(real);
        let mut carry = 0.0;
        for f in &tl.frames {
            if f.frametime_ms < limit {
                carry += f.frametime_ms;
            } else {
                out.push(FramePoint {
                    frametime_ms: f.frametime_ms + carry,
                    ..*f
                });
                carry = 0.0;
            }
        }
        (out, FrameGenSeries::Real)
    };
    Some((
        Timeline {
            frames,
            ..tl.clone()
        },
        FrameGenInfo {
            presented_fps,
            real_fps,
            factor: presented_fps / real_fps.max(f64::EPSILON),
            series,
        },
    ))
}
