//! Прогрів: чи встигла температура GPU вийти на плато перед першим прогоном.
//!
//! Береться нахил прямої за останні [`WINDOW_S`] секунд знімків. Плато — нахил менший за
//! [`STABLE_C_PER_MIN`]. Якщо плато немає довше за [`WARMUP_MAX_S`], тест починається все одно
//! (з попередженням), щоб не чекати вічно.

use serde::{Deserialize, Serialize};

/// Вікно оцінки нахилу, с.
pub const WINDOW_S: f64 = 60.0;
/// Нахил, нижче за який температура вважається сталою, °C/хв.
pub const STABLE_C_PER_MIN: f64 = 0.5;
/// Найдовший прогрів, с.
pub const WARMUP_MAX_S: f64 = 300.0;

/// Стан прогріву.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Thermal {
    /// Температура на плато.
    pub stable: bool,
    /// Нахил за останнє вікно, °C/хв (`None` — замало знімків).
    pub slope_c_per_min: Option<f64>,
    /// Остання температура, °C.
    pub last_c: Option<f64>,
}

/// Оцінює прогрів за знімками `(секунди від старту, °C)`, упорядкованими за часом.
pub fn thermal_state(samples: &[(f64, f64)]) -> Thermal {
    let last = samples.last().copied();
    let Some((t_end, last_c)) = last else {
        return Thermal {
            stable: false,
            slope_c_per_min: None,
            last_c: None,
        };
    };
    let window: Vec<(f64, f64)> = samples
        .iter()
        .copied()
        .filter(|(t, c)| *t >= t_end - WINDOW_S && t.is_finite() && c.is_finite())
        .collect();
    let span = window.first().map_or(0.0, |(t0, _)| t_end - t0);
    let slope = (window.len() >= 3 && span > 0.0).then(|| {
        let n = window.len() as f64;
        let mt = window.iter().map(|p| p.0).sum::<f64>() / n;
        let mc = window.iter().map(|p| p.1).sum::<f64>() / n;
        let sxy: f64 = window.iter().map(|(t, c)| (t - mt) * (c - mc)).sum();
        let sxx: f64 = window.iter().map(|(t, _)| (t - mt).powi(2)).sum();
        if sxx > 0.0 {
            sxy / sxx * 60.0
        } else {
            0.0
        }
    });
    Thermal {
        // Плато лише за повним вікном: інакше короткий відрізок «рівної» температури обманює.
        stable: span >= WINDOW_S * 0.95 && slope.is_some_and(|s| s.abs() < STABLE_C_PER_MIN),
        slope_c_per_min: slope,
        last_c: Some(last_c),
    }
}
