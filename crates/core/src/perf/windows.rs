//! PDH-запит з кількома лічильниками.
#![allow(unsafe_code)]

use windows::core::HSTRING;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};

/// `ERROR_SUCCESS` для функцій PDH.
const OK: u32 = 0;
/// `PDH_MORE_DATA`: буфер замалий, потрібний розмір повернуто.
const MORE_DATA: u32 = 0x8000_07D2;

pub(super) struct PdhQuery {
    query: PDH_HQUERY,
    counters: Vec<PDH_HCOUNTER>,
    primed: bool,
}

// SAFETY: дескриптори PDH — непрозорі значення, не прив'язані до потоку; запит використовується
// лише через `&mut self`, тобто з одного потоку в кожен момент.
unsafe impl Send for PdhQuery {}

impl PdhQuery {
    /// Запит із лічильниками за англійськими шляхами (можна з `(*)` для всіх екземплярів).
    /// `None`, якщо хоч один лічильник недоступний.
    pub(super) fn open(paths: &[&str]) -> Option<Self> {
        let mut query = PDH_HQUERY::default();
        // SAFETY: локальний вихідний параметр; джерело даних — живий комп'ютер (null).
        if unsafe { PdhOpenQueryW(None, 0, &mut query) } != OK {
            return None;
        }
        let mut this = Self {
            query,
            counters: Vec::with_capacity(paths.len()),
            primed: false,
        };
        for path in paths {
            let mut counter = PDH_HCOUNTER::default();
            // SAFETY: запит живий (закриється в Drop); шлях — нуль-термінований HSTRING.
            if unsafe { PdhAddEnglishCounterW(query, &HSTRING::from(*path), 0, &mut counter) } != OK
            {
                return None;
            }
            this.counters.push(counter);
        }
        Some(this)
    }

    /// Новий вимір. `false` — помилка або це перший вимір (швидкісним лічильникам потрібні два).
    fn collect(&mut self) -> bool {
        // SAFETY: запит живий до Drop.
        if unsafe { PdhCollectQueryData(self.query) } != OK {
            return false;
        }
        if !self.primed {
            self.primed = true;
            return false;
        }
        true
    }

    /// Значення першого лічильника з моменту попереднього виміру.
    pub(super) fn sample(&mut self) -> Option<f64> {
        if !self.collect() {
            return None;
        }
        let counter = *self.counters.first()?;
        let mut value = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: лічильник належить запиту; вихідна структура локальна.
        if unsafe { PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut value) } != OK {
            return None;
        }
        // SAFETY: запитано PDH_FMT_DOUBLE, тож активне поле — doubleValue.
        let v = unsafe { value.Anonymous.doubleValue };
        v.is_finite().then_some(v.max(0.0))
    }

    /// Значення кожного екземпляра для кожного лічильника: `[лічильник][(ім'я, значення)]`.
    pub(super) fn sample_instances(&mut self) -> Option<Vec<Vec<(String, f64)>>> {
        if !self.collect() {
            return None;
        }
        self.counters.iter().map(|c| instances(*c)).collect()
    }
}

/// Екземпляри одного лічильника з останнього виміру.
fn instances(counter: PDH_HCOUNTER) -> Option<Vec<(String, f64)>> {
    let (mut size, mut count) = (0u32, 0u32);
    // SAFETY: запит розміру — буфер не передаємо.
    let r = unsafe {
        PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None)
    };
    if r != MORE_DATA || size == 0 {
        return None;
    }
    // Буфер з u64 — вирівнювання не гірше, ніж потрібно структурам (вказівник + f64).
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
    // SAFETY: буфер щонайменше `size` байт і вирівняний на 8; PDH заповнює `count` елементів,
    // а рядки імен кладе в той самий буфер.
    let r = unsafe {
        PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items))
    };
    if r != OK
        || (count as usize) * std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>() > buf.len() * 8
    {
        return None;
    }
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        // SAFETY: i < count, елементи лежать у буфері (перевірено вище), буфер живий.
        let item = unsafe { &*items.add(i) };
        // SAFETY: szName вказує на нуль-термінований рядок у тому самому буфері.
        let name = unsafe { item.szName.to_string() }.unwrap_or_default();
        // SAFETY: запитано PDH_FMT_DOUBLE, тож активне поле — doubleValue.
        let v = unsafe { item.FmtValue.Anonymous.doubleValue };
        if v.is_finite() {
            out.push((name, v.max(0.0)));
        }
    }
    Some(out)
}

impl Drop for PdhQuery {
    fn drop(&mut self) {
        // SAFETY: запит відкрито в `open`, закривається один раз (разом із лічильниками).
        unsafe { PdhCloseQuery(self.query) };
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DpcCounter, HardFaultCounter};

    #[test]
    fn hard_fault_counter_reads_real_value() {
        let mut c = HardFaultCounter::open().expect(r"лічильник \Memory\Pages Input/sec");
        assert_eq!(
            c.sample(),
            None,
            "перший вимір лише ініціалізує швидкісний лічильник"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
        let v = c.sample().expect("друге значення");
        assert!(v >= 0.0);
    }

    #[test]
    fn dpc_counter_reads_real_value() {
        let mut c = DpcCounter::open().expect("лічильники DPC/переривань");
        assert_eq!(c.sample(), None);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let v = c.sample().expect("друге значення");
        assert!((0.0..=100.0).contains(&v), "{v}");
    }
}
