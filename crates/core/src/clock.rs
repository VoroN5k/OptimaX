//! Спільний монотонний годинник на базі QPC.
//!
//! PresentMon (`--qpc_time`) пише час кадрів у тиках `QueryPerformanceCounter`; метрики беруть
//! той самий лічильник, тому кадри й системні знімки лягають на одну шкалу часу без зсуву.

/// Мітка часу в тиках QPC (на не-Windows — наносекунди від першого виклику).
pub type QpcTicks = i64;

/// Поточне значення лічильника.
pub fn qpc_now() -> QpcTicks {
    imp::now()
}

/// Кількість тиків за секунду.
pub fn qpc_frequency() -> i64 {
    imp::frequency()
}

/// Різниця двох міток у мілісекундах.
pub fn ticks_to_ms(delta: QpcTicks, frequency: i64) -> f64 {
    if frequency <= 0 {
        return 0.0;
    }
    delta as f64 * 1000.0 / frequency as f64
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    pub(super) fn now() -> i64 {
        let mut v = 0i64;
        // SAFETY: вказівник на локальну змінну; на Windows XP+ функція не завершується помилкою.
        let _ = unsafe { QueryPerformanceCounter(&mut v) };
        v
    }

    pub(super) fn frequency() -> i64 {
        let mut v = 0i64;
        // SAFETY: як вище; частота незмінна з моменту завантаження ОС.
        let _ = unsafe { QueryPerformanceFrequency(&mut v) };
        v
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::OnceLock;
    use std::time::Instant;

    fn origin() -> Instant {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        *ORIGIN.get_or_init(Instant::now)
    }

    pub(super) fn now() -> i64 {
        i64::try_from(origin().elapsed().as_nanos()).unwrap_or(i64::MAX)
    }

    pub(super) fn frequency() -> i64 {
        1_000_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic_and_converts() {
        let a = qpc_now();
        let b = qpc_now();
        assert!(b >= a);
        assert!(qpc_frequency() > 0);
        assert_eq!(ticks_to_ms(10_000_000, 10_000_000), 1000.0);
        assert_eq!(ticks_to_ms(5, 0), 0.0);
    }
}
