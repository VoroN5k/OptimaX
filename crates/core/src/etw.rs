//! Трасування DPC і переривань драйверів через ETW (події ядра `PerfInfo`).
//!
//! [`DpcTracer`] запускає окрему системну сесію ETW (`EVENT_TRACE_SYSTEM_LOGGER_MODE`, Windows 8+,
//! не заважає PresentMon і іншим інструментам) з прапорцями `DPC` та `INTERRUPT`. Кожна подія
//! містить адресу процедури драйвера й час її старту; час події — кінець виконання. За адресою
//! визначаємо драйвер (список модулів ядра `EnumDeviceDrivers`).
//!
//! Подій бувають десятки тисяч за секунду, тож нічого не зберігаємо поштучно, крім довгих
//! (≥ [`LONG_DPC_US`] мкс): лише сумарна статистика по процедурах. Потрібні права адміністратора.

#[cfg(windows)]
mod windows;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Події, довші за це (мкс), зберігаються поштучно — для збігу з піками frametime.
pub const LONG_DPC_US: f64 = 100.0;
/// Скільки довгих подій зберігати щонайбільше (захист пам'яті).
const MAX_LONG: usize = 50_000;
/// Подія, довша за це, вважається збоєм часу й відкидається.
const MAX_SANE_US: f64 = 1_000_000.0;
/// Процедура далі від початку модуля, ніж це, не приписується модулю (розмір модуля невідомий).
const MAX_MODULE_SPAN: u64 = 256 * 1024 * 1024;

/// Вид події.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DpcKind {
    /// Відкладений виклик процедури (DPC, зокрема таймерний і потоковий).
    Dpc,
    /// Обробник переривання (ISR).
    Isr,
}

/// Підсумок по одному драйверу.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverDpc {
    /// Ім'я модуля (`nvlddmkm.sys`); `None` — адреса поза відомими модулями.
    pub name: Option<String>,
    /// Кількість DPC.
    pub dpc_count: u64,
    /// Кількість переривань.
    pub isr_count: u64,
    /// Сумарний час виконання, мкс.
    pub total_us: f64,
    /// Найдовше виконання, мкс.
    pub max_us: f64,
}

/// Довга подія.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LongDpc {
    /// Початок виконання, тики QPC (та сама шкала, що й кадри PresentMon).
    pub start_qpc: i64,
    /// Тривалість, мкс.
    pub dur_us: f32,
    /// Індекс у [`DpcTrace::drivers`].
    pub driver: u16,
    /// Вид події.
    pub kind: DpcKind,
}

/// Результат трасування.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DpcTrace {
    /// Драйвери за спаданням сумарного часу.
    pub drivers: Vec<DriverDpc>,
    /// Довгі події за часом.
    pub long: Vec<LongDpc>,
    /// Усього подій.
    pub events: u64,
    /// Подій, втрачених ETW (буфери не встигли).
    pub lost: u64,
    /// Довгих подій, що не вмістились у ліміт.
    pub long_dropped: u64,
}

/// Подія, як її повертає ETW.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
struct RawEvent {
    start_qpc: i64,
    end_qpc: i64,
    routine: u64,
    kind: DpcKind,
}

#[derive(Debug, Default, Clone, Copy)]
struct RoutineStat {
    dpc: u64,
    isr: u64,
    total_ticks: i64,
    max_ticks: i64,
}

/// Збирає статистику по процедурах під час трасування.
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
struct Collector {
    freq: i64,
    long_ticks: i64,
    max_ticks: i64,
    routines: HashMap<u64, RoutineStat>,
    long: Vec<(i64, i64, u64, DpcKind)>,
    long_dropped: u64,
    events: u64,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Collector {
    fn new(freq: i64) -> Self {
        let ticks = |us: f64| (us * freq as f64 / 1e6) as i64;
        Self {
            freq,
            long_ticks: ticks(LONG_DPC_US).max(1),
            max_ticks: ticks(MAX_SANE_US),
            routines: HashMap::with_capacity(512),
            long: Vec::with_capacity(4096),
            long_dropped: 0,
            events: 0,
        }
    }

    fn push(&mut self, e: RawEvent) {
        let dur = e.end_qpc - e.start_qpc;
        if dur < 0 || dur > self.max_ticks {
            return;
        }
        self.events += 1;
        let s = self.routines.entry(e.routine).or_default();
        match e.kind {
            DpcKind::Dpc => s.dpc += 1,
            DpcKind::Isr => s.isr += 1,
        }
        s.total_ticks += dur;
        s.max_ticks = s.max_ticks.max(dur);
        if dur >= self.long_ticks {
            if self.long.len() < MAX_LONG {
                self.long.push((e.start_qpc, dur, e.routine, e.kind));
            } else {
                self.long_dropped += 1;
            }
        }
    }

    /// Підсумок із приписуванням процедур модулям `(база, ім'я)`.
    fn finish(self, modules: &[(u64, String)], lost: u64) -> DpcTrace {
        let mut mods: Vec<&(u64, String)> = modules.iter().filter(|(b, _)| *b != 0).collect();
        mods.sort_by_key(|(b, _)| *b);
        let owner = |routine: u64| -> Option<&str> {
            let i = mods.partition_point(|(b, _)| *b <= routine);
            let (base, name) = mods.get(i.checked_sub(1)?)?;
            (routine - base < MAX_MODULE_SPAN).then_some(name.as_str())
        };
        let us = |ticks: i64| ticks as f64 * 1e6 / self.freq.max(1) as f64;

        let mut by_name: HashMap<Option<&str>, DriverDpc> = HashMap::new();
        let mut routine_driver: HashMap<u64, Option<&str>> = HashMap::new();
        for (routine, s) in &self.routines {
            let name = owner(*routine);
            routine_driver.insert(*routine, name);
            let d = by_name.entry(name).or_insert_with(|| DriverDpc {
                name: name.map(str::to_string),
                dpc_count: 0,
                isr_count: 0,
                total_us: 0.0,
                max_us: 0.0,
            });
            d.dpc_count += s.dpc;
            d.isr_count += s.isr;
            d.total_us += us(s.total_ticks);
            d.max_us = d.max_us.max(us(s.max_ticks));
        }
        let mut drivers: Vec<DriverDpc> = by_name.into_values().collect();
        drivers.sort_by(|a, b| b.total_us.total_cmp(&a.total_us));
        let index: HashMap<Option<&str>, u16> = drivers
            .iter()
            .enumerate()
            .map(|(i, d)| (d.name.as_deref(), u16::try_from(i).unwrap_or(u16::MAX)))
            .collect();

        let mut long: Vec<LongDpc> = self
            .long
            .iter()
            .map(|(start, dur, routine, kind)| LongDpc {
                start_qpc: *start,
                dur_us: us(*dur) as f32,
                driver: routine_driver
                    .get(routine)
                    .and_then(|n| index.get(n))
                    .copied()
                    .unwrap_or(u16::MAX),
                kind: *kind,
            })
            .collect();
        long.sort_by_key(|e| e.start_qpc);
        DpcTrace {
            drivers,
            long,
            events: self.events,
            lost,
            long_dropped: self.long_dropped,
        }
    }
}

/// Помилка запуску трасування.
#[derive(Debug, thiserror::Error)]
pub enum EtwError {
    /// Не Windows.
    #[error("трасування ETW доступне лише на Windows")]
    Unsupported,
    /// Помилка WinAPI (код Win32).
    #[error("{call}: помилка {code}")]
    Api {
        /// Функція, що повернула помилку.
        call: &'static str,
        /// Код помилки Win32.
        code: u32,
    },
}

/// Запущена сесія трасування DPC. Зупиняється [`DpcTracer::stop`] або при знищенні.
pub struct DpcTracer {
    #[cfg(windows)]
    inner: windows::Session,
}

impl DpcTracer {
    /// Запускає сесію. `freq` — частота QPC (для переведення тиків у мкс).
    pub fn start(freq: i64) -> Result<Self, EtwError> {
        #[cfg(windows)]
        {
            windows::Session::start(Collector::new(freq)).map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = freq;
            Err(EtwError::Unsupported)
        }
    }

    /// Зупиняє сесію й повертає підсумок. `None`, якщо потік обробки не завершився.
    pub fn stop(self) -> Option<DpcTrace> {
        #[cfg(windows)]
        {
            let (collector, lost) = self.inner.stop()?;
            Some(collector.finish(&windows::kernel_modules(), lost))
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

impl std::fmt::Debug for DpcTracer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DpcTracer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(start: i64, dur: i64, routine: u64, kind: DpcKind) -> RawEvent {
        RawEvent {
            start_qpc: start,
            end_qpc: start + dur,
            routine,
            kind,
        }
    }

    #[test]
    fn collector_attributes_routines_to_modules() {
        // 10 МГц: 1 тик = 0,1 мкс.
        let mut c = Collector::new(10_000_000);
        c.push(ev(0, 500, 0x1000_0100, DpcKind::Dpc)); // 50 мкс, gpu
        c.push(ev(1000, 20_000, 0x1000_0200, DpcKind::Dpc)); // 2 мс, gpu — довга
        c.push(ev(2000, 3_000, 0x2000_0010, DpcKind::Isr)); // 300 мкс, net — довга
        c.push(ev(3000, -5, 0x2000_0010, DpcKind::Isr)); // збій часу — відкидається
        c.push(ev(4000, 10, 0x9000_0000_0000, DpcKind::Dpc)); // поза модулями
        let modules = vec![
            (0x2000_0000, "net.sys".to_string()),
            (0x1000_0000, "gpu.sys".to_string()),
            (0, "zero.sys".to_string()),
        ];
        let t = c.finish(&modules, 7);
        assert_eq!(t.events, 4);
        assert_eq!(t.lost, 7);
        assert_eq!(t.drivers[0].name.as_deref(), Some("gpu.sys"));
        assert_eq!(t.drivers[0].dpc_count, 2);
        assert!((t.drivers[0].total_us - 2050.0).abs() < 1e-6);
        assert!((t.drivers[0].max_us - 2000.0).abs() < 1e-6);
        assert_eq!(t.drivers[1].name.as_deref(), Some("net.sys"));
        assert_eq!(t.drivers[1].isr_count, 1);
        let unknown = t.drivers.iter().find(|d| d.name.is_none());
        assert!(unknown.is_some_and(|d| d.dpc_count == 1));
        assert_eq!(t.long.len(), 2);
        assert_eq!(t.long[0].start_qpc, 1000);
        assert_eq!(
            t.drivers[usize::from(t.long[0].driver)].name.as_deref(),
            Some("gpu.sys")
        );
        assert_eq!(t.long[1].kind, DpcKind::Isr);
        assert!((t.long[1].dur_us - 300.0).abs() < 1e-3);
    }
}
