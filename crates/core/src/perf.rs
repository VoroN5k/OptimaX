//! Лічильники продуктивності Windows (PDH), потрібні діагностиці.
//!
//! [`HardFaultCounter`] — `\Memory\Pages Input/sec`: скільки сторінок пам'яті за секунду
//! читається з диска, щоб обслужити жорсткі page faults (підкачка або файли, відображені в
//! пам'ять). Лічильник додається за англійською назвою (`PdhAddEnglishCounterW`), тож не
//! залежить від мови Windows. Це швидкісний лічильник: перше значення з'являється лише після
//! другого виміру.
//!
//! [`DpcCounter`] — частка часу, яку ядро CPU витрачає на DPC і переривання драйверів
//! (`\Processor Information(*)\% DPC Time` + `% Interrupt Time`), на найзайнятішому ядрі.
//! Драйвер, що надовго займає ядро, зупиняє на цей час і потік гри — звідси ривки, коли
//! CPU, диск і пам'ять у нормі. Який саме драйвер — ці лічильники не кажуть.

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
            windows::PdhQuery::open(&[r"\Memory\Pages Input/sec"]).map(|inner| Self { inner })
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

/// Час на DPC і переривання на найзайнятішому ядрі, %.
pub struct DpcCounter {
    #[cfg(windows)]
    inner: windows::PdhQuery,
}

impl DpcCounter {
    /// Відкриває лічильники. `None`, якщо PDH недоступний.
    pub fn open() -> Option<Self> {
        #[cfg(windows)]
        {
            windows::PdhQuery::open(&[
                r"\Processor Information(*)\% DPC Time",
                r"\Processor Information(*)\% Interrupt Time",
            ])
            .map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// DPC + переривання на найзайнятішому ядрі з моменту попереднього виклику, %.
    /// Перший виклик дає `None`.
    pub fn sample(&mut self) -> Option<f64> {
        #[cfg(windows)]
        {
            let sets = self.inner.sample_instances()?;
            Some(busiest_core(&sets))
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

impl std::fmt::Debug for DpcCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DpcCounter")
    }
}

/// Сума лічильників по кожному ядру (без підсумкових екземплярів `_Total`), максимум по ядрах.
#[cfg_attr(not(windows), allow(dead_code))]
fn busiest_core(sets: &[Vec<(String, f64)>]) -> f64 {
    let mut per_core: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for set in sets {
        for (name, v) in set {
            if !name.contains("_Total") {
                *per_core.entry(name.as_str()).or_default() += v;
            }
        }
    }
    per_core.values().copied().fold(0.0, f64::max).min(100.0)
}

impl std::fmt::Debug for HardFaultCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HardFaultCounter")
    }
}

#[cfg(test)]
mod tests {
    use super::busiest_core;

    #[test]
    fn busiest_core_sums_counters_and_skips_totals() {
        let dpc = vec![
            ("0,0".to_string(), 1.0),
            ("0,1".to_string(), 4.0),
            ("0,_Total".to_string(), 2.5),
            ("_Total".to_string(), 2.5),
        ];
        let irq = vec![("0,0".to_string(), 5.0), ("0,1".to_string(), 0.5)];
        assert_eq!(busiest_core(&[dpc, irq]), 6.0);
        assert_eq!(busiest_core(&[]), 0.0);
    }
}
