//! PDH-запит з одним лічильником.
#![allow(unsafe_code)]

use windows::core::HSTRING;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
    PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};

/// `ERROR_SUCCESS` для функцій PDH.
const OK: u32 = 0;

pub(super) struct PdhQuery {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
    primed: bool,
}

// SAFETY: дескриптори PDH — непрозорі значення, не прив'язані до потоку; запит використовується
// лише через `&mut self`, тобто з одного потоку в кожен момент.
unsafe impl Send for PdhQuery {}

impl PdhQuery {
    pub(super) fn open(path: &str) -> Option<Self> {
        let mut query = PDH_HQUERY::default();
        // SAFETY: локальний вихідний параметр; джерело даних — живий комп'ютер (null).
        if unsafe { PdhOpenQueryW(None, 0, &mut query) } != OK {
            return None;
        }
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: запит щойно відкрито; шлях — нуль-термінований HSTRING.
        if unsafe { PdhAddEnglishCounterW(query, &HSTRING::from(path), 0, &mut counter) } != OK {
            // SAFETY: закриваємо запит, який щойно відкрили.
            unsafe { PdhCloseQuery(query) };
            return None;
        }
        Some(Self {
            query,
            counter,
            primed: false,
        })
    }

    pub(super) fn sample(&mut self) -> Option<f64> {
        // SAFETY: запит живий до Drop.
        if unsafe { PdhCollectQueryData(self.query) } != OK {
            return None;
        }
        if !self.primed {
            // Швидкісному лічильнику потрібні два виміри.
            self.primed = true;
            return None;
        }
        let mut value = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: лічильник належить запиту; вихідна структура локальна.
        if unsafe { PdhGetFormattedCounterValue(self.counter, PDH_FMT_DOUBLE, None, &mut value) }
            != OK
        {
            return None;
        }
        // SAFETY: запитано PDH_FMT_DOUBLE, тож активне поле — doubleValue.
        let v = unsafe { value.Anonymous.doubleValue };
        v.is_finite().then_some(v.max(0.0))
    }
}

impl Drop for PdhQuery {
    fn drop(&mut self) {
        // SAFETY: запит відкрито в `open`, закривається один раз (разом із лічильником).
        unsafe { PdhCloseQuery(self.query) };
    }
}

#[cfg(test)]
mod tests {
    use super::super::HardFaultCounter;

    #[test]
    fn hard_fault_counter_reads_real_value() {
        let mut c = HardFaultCounter::open().expect("лічильник \\Memory\\Pages Input/sec");
        assert_eq!(
            c.sample(),
            None,
            "перший вимір лише ініціалізує швидкісний лічильник"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
        let v = c.sample().expect("друге значення");
        assert!(v >= 0.0);
    }
}
