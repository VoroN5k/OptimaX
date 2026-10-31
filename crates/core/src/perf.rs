//! Лічильники продуктивності Windows (PDH), потрібні діагностиці.
//!
//! [`HardFaultCounter`] — `\Memory\Pages Input/sec`: скільки сторінок пам'яті за секунду
//! читається з диска, щоб обслужити жорсткі page faults (підкачка або файли, відображені в
//! пам'ять). Лічильник додається за англійською назвою (`PdhAddEnglishCounterW`), тож не
//! залежить від мови Windows. Це швидкісний лічильник: перше значення з'являється лише після
//! другого виміру.

#[cfg(windows)]
mod windows;

/// Сторінки, прочитані з диска через жорсткі page faults, за секунду.
pub struct HardFaultCounter {
    #[cfg(windows)]
    inner: windows::PdhQuery,
}

impl HardFaultCounter {
    /// Відкриває лічильник. `None`, якщо PDH недоступний (немає лічильника, служба вимкнена).
    pub fn open() -> Option<Self> {
        #[cfg(windows)]
        {
            windows::PdhQuery::open(r"\Memory\Pages Input/sec").map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// Значення з моменту попереднього виклику. Перший виклик дає `None`.
    pub fn sample(&mut self) -> Option<f64> {
        #[cfg(windows)]
        {
            self.inner.sample()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

impl std::fmt::Debug for HardFaultCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HardFaultCounter")
    }
}
