//! Виміри вартості апскейлу: GPU-час кадру, затримка, навантаження на CPU.
//! Чиста логіка (накопичення й зведення) — покрита тестами; самі заміри — у платформному модулі.

use serde::{Deserialize, Serialize};

/// Скільки останніх замірів тримати (≈5 с при 200 кадрах/с).
pub const WINDOW: usize = 1024;

/// Зведення ряду замірів у мілісекундах.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
// В інтерфейсі вже є `Summary` (статистика A/B-тесту) — інше ім'я, щоб файли типів не збігались.
#[cfg_attr(feature = "ts", ts(rename = "TimingSummary"))]
pub struct Summary {
    /// Медіана.
    pub median: f64,
    /// 95-й перцентиль.
    pub p95: f64,
    /// Максимум.
    pub max: f64,
    /// Кількість замірів.
    pub count: u32,
}

/// Кільцевий буфер останніх замірів.
#[derive(Debug, Clone)]
pub struct Samples {
    values: Vec<f64>,
    next: usize,
    capacity: usize,
}

impl Samples {
    /// Порожній буфер на `capacity` замірів (щонайменше 1).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            values: Vec::with_capacity(capacity),
            next: 0,
            capacity,
        }
    }

    /// Додає замір; некоректні (від'ємні, NaN, нескінченні) відкидаються.
    pub fn push(&mut self, value: f64) {
        if !value.is_finite() || value < 0.0 {
            return;
        }
        if self.values.len() < self.capacity {
            self.values.push(value);
        } else {
            self.values[self.next] = value;
        }
        self.next = (self.next + 1) % self.capacity;
    }

    /// Чи порожньо.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Частка замірів понад `threshold`, % (`None`, якщо замірів немає).
    pub fn share_above(&self, threshold: f64) -> Option<f64> {
        if self.values.is_empty() {
            return None;
        }
        let above = self.values.iter().filter(|v| **v > threshold).count();
        Some(above as f64 / self.values.len() as f64 * 100.0)
    }

    /// Зведення (`None`, якщо замірів немає).
    pub fn summary(&self) -> Option<Summary> {
        if self.values.is_empty() {
            return None;
        }
        let mut sorted = self.values.clone();
        sorted.sort_by(f64::total_cmp);
        Some(Summary {
            median: percentile(&sorted, 50.0),
            p95: percentile(&sorted, 95.0),
            max: sorted.last().copied().unwrap_or_default(),
            count: u32::try_from(sorted.len()).unwrap_or(u32::MAX),
        })
    }
}

/// Перцентиль відсортованого ряду (лінійна інтерполяція між сусідніми значеннями).
fn percentile(sorted: &[f64], p: f64) -> f64 {
    match sorted.len() {
        0 => 0.0,
        1 => sorted[0],
        n => {
            let rank = (p / 100.0).clamp(0.0, 1.0) * (n - 1) as f64;
            let lo = rank.floor() as usize;
            let hi = rank.ceil() as usize;
            let t = rank - lo as f64;
            sorted[lo] + (sorted[hi] - sorted[lo]) * t
        }
    }
}

/// Тики QPC → одиниці по 100 нс (у них `SystemRelativeTime` кадру WGC).
pub fn qpc_to_100ns(ticks: i64, frequency: i64) -> i64 {
    if frequency <= 0 {
        return 0;
    }
    i64::try_from(i128::from(ticks) * 10_000_000 / i128::from(frequency)).unwrap_or(i64::MAX)
}

/// Різниця двох моментів у 100 нс → мілісекунди.
pub fn hns_delta_ms(from: i64, to: i64) -> f64 {
    (to - from) as f64 / 10_000.0
}

/// Поріг «кадр запізнився»: показ на V-Sync пізніше, ніж без апскейлу. Затримка кратна періоду
/// оновлення (щонайменше ~4 мс навіть на 240 Гц), тож 1 мс відділяє «той самий V-Sync» з запасом.
pub const LATE_THRESHOLD_MS: f64 = 1.0;

/// Запас до запланованого V-Sync кадру гри, менше якого кадр уже не встигає на свій показ
/// (100 нс). Такий кадр краще пропустити й показати наступний — див. [`should_skip_frame`].
pub const VSYNC_MARGIN_HNS: i64 = 10_000;

/// Чи пропустити новий кадр гри замість показу.
///
/// Показ кадру, що вже не встигає на свій V-Sync, займає чергу показу до наступного V-Sync —
/// і наступний кадр гри теж запізнюється, і так по колу (стійке відставання на кадр, виміряне
/// на кроці 4). Пропуск одного кадру повертає синхронізацію. Поспіль пропускаємо не більше
/// одного кадру, щоб на перевантаженій системі не перестати показувати взагалі.
pub fn should_skip_frame(now_hns: i64, scheduled_hns: i64, skipped_last: bool) -> bool {
    !skipped_last && scheduled_hns != 0 && now_hns > scheduled_hns - VSYNC_MARGIN_HNS
}

/// Частка одного ядра CPU у відсотках: процесорний час потоку за інтервал / тривалість інтервалу.
/// Обидва значення — у 100 нс (FILETIME).
pub fn cpu_percent(cpu_delta: u64, wall_delta: u64) -> f64 {
    if wall_delta == 0 {
        return 0.0;
    }
    cpu_delta as f64 / wall_delta as f64 * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_of_simple_series() -> Result<(), &'static str> {
        let mut s = Samples::new(100);
        for v in 1..=100 {
            s.push(f64::from(v));
        }
        let sum = s.summary().ok_or("empty")?;
        assert!((sum.median - 50.5).abs() < 1e-9);
        assert!((sum.p95 - 95.05).abs() < 1e-9);
        assert_eq!(sum.max, 100.0);
        assert_eq!(sum.count, 100);
        Ok(())
    }

    #[test]
    fn ring_keeps_only_latest() -> Result<(), &'static str> {
        let mut s = Samples::new(3);
        for v in [100.0, 1.0, 2.0, 3.0] {
            s.push(v);
        }
        let sum = s.summary().ok_or("empty")?;
        assert_eq!(sum.max, 3.0);
        assert_eq!(sum.median, 2.0);
        assert_eq!(sum.count, 3);
        Ok(())
    }

    #[test]
    fn share_above_threshold() {
        let mut s = Samples::new(10);
        assert_eq!(s.share_above(1.0), None);
        for v in [0.0, 0.0, 0.0, 5.0] {
            s.push(v);
        }
        assert_eq!(s.share_above(1.0), Some(25.0));
    }

    #[test]
    fn bad_values_are_ignored() {
        let mut s = Samples::new(4);
        s.push(f64::NAN);
        s.push(-1.0);
        s.push(f64::INFINITY);
        assert!(s.is_empty());
        assert!(s.summary().is_none());
        s.push(0.5);
        assert_eq!(s.summary().map(|x| x.count), Some(1));
    }

    #[test]
    fn single_value_summary() {
        let mut s = Samples::new(0);
        s.push(1.25);
        let sum = s.summary().unwrap_or_default();
        assert_eq!((sum.median, sum.p95, sum.max), (1.25, 1.25, 1.25));
    }

    #[test]
    fn qpc_conversion() {
        // 10 МГц QPC — тики вже в 100 нс.
        assert_eq!(qpc_to_100ns(123_456, 10_000_000), 123_456);
        // 3 МГц: 3 000 000 тиків = 1 с = 10 000 000 × 100 нс.
        assert_eq!(qpc_to_100ns(3_000_000, 3_000_000), 10_000_000);
        assert_eq!(qpc_to_100ns(5, 0), 0);
        assert!((hns_delta_ms(10_000, 52_000) - 4.2).abs() < 1e-9);
    }

    #[test]
    fn late_frames_are_skipped_once() {
        let vsync = 1_000_000;
        // 4,4 мс до V-Sync — встигає.
        assert!(!should_skip_frame(vsync - 44_000, vsync, false));
        // Уже на V-Sync або за 0,5 мс до нього — не встигає.
        assert!(should_skip_frame(vsync, vsync, false));
        assert!(should_skip_frame(vsync - 5_000, vsync, false));
        // Двічі поспіль не пропускаємо.
        assert!(!should_skip_frame(vsync, vsync, true));
        // Без мітки часу кадру — не пропускаємо.
        assert!(!should_skip_frame(vsync, 0, false));
    }

    #[test]
    fn cpu_share_of_one_core() {
        assert_eq!(cpu_percent(5_000_000, 10_000_000), 50.0);
        assert_eq!(cpu_percent(1, 0), 0.0);
    }
}
