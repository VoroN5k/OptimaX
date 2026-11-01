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
use std::collections::{HashMap, VecDeque};

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

/// Статистика по процедурах (адреса → лічильники).
type Routines = HashMap<u64, RoutineStat>;

/// Довга подія до приписування модулю: (старт QPC, тривалість у тиках, процедура, вид).
type RawLong = (i64, i64, u64, DpcKind);

/// Збирає статистику по процедурах під час трасування.
///
/// Звичайний режим — підсумок за всю сесію. Кільцевий ([`Collector::ring`]) — лише останні
/// `keep` тиків: статистика по секундних кошиках і довгі події, старіші за вікно, відкидаються,
/// тож пам'ять не росте за довгої гри, а зріз за будь-яке вікно в межах `keep` доступний на льоту.
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
struct Collector {
    freq: i64,
    long_ticks: i64,
    max_ticks: i64,
    /// Кільцевий режим: скільки тиків історії тримати.
    keep_ticks: Option<i64>,
    routines: Routines,
    /// Кільцевий режим: (номер секунди QPC, статистика за цю секунду), за зростанням.
    buckets: VecDeque<(i64, Routines)>,
    long: VecDeque<RawLong>,
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
            keep_ticks: None,
            routines: HashMap::with_capacity(512),
            buckets: VecDeque::new(),
            long: VecDeque::with_capacity(4096),
            long_dropped: 0,
            events: 0,
        }
    }

    /// Кільцевий колектор: тримає лише останні `keep_secs` секунд.
    fn ring(freq: i64, keep_secs: u32) -> Self {
        Self {
            keep_ticks: Some(i64::from(keep_secs.max(1)) * freq.max(1)),
            ..Self::new(freq)
        }
    }

    fn push(&mut self, e: RawEvent) {
        let dur = e.end_qpc - e.start_qpc;
        if dur < 0 || dur > self.max_ticks {
            return;
        }
        self.events += 1;
        let stats = match self.keep_ticks {
            None => &mut self.routines,
            Some(keep) => {
                let freq = self.freq.max(1);
                let second = e.start_qpc.div_euclid(freq);
                if self.buckets.back().is_none_or(|(s, _)| *s < second) {
                    let horizon = e.end_qpc - keep;
                    let oldest = horizon.div_euclid(freq);
                    while self.buckets.front().is_some_and(|(s, _)| *s < oldest) {
                        self.buckets.pop_front();
                    }
                    while self.long.front().is_some_and(|l| l.0 < horizon) {
                        self.long.pop_front();
                    }
                    self.buckets.push_back((second, HashMap::with_capacity(64)));
                }
                // Події з різних ядер приходять не строго за часом старту — кладемо в кошик
                // своєї секунди, якщо він ще є (інакше в найстаріший).
                let i = self
                    .buckets
                    .iter()
                    .rposition(|(s, _)| *s <= second)
                    .unwrap_or(0);
                match self.buckets.get_mut(i) {
                    Some((_, r)) => r,
                    None => return,
                }
            }
        };
        let s = stats.entry(e.routine).or_default();
        match e.kind {
            DpcKind::Dpc => s.dpc += 1,
            DpcKind::Isr => s.isr += 1,
        }
        s.total_ticks += dur;
        s.max_ticks = s.max_ticks.max(dur);
        if dur >= self.long_ticks {
            if self.long.len() < MAX_LONG {
                self.long.push_back((e.start_qpc, dur, e.routine, e.kind));
            } else {
                self.long_dropped += 1;
            }
        }
    }

    /// Зріз за `[from_qpc, to_qpc]`. У звичайному режимі — уся сесія (вікно ігнорується).
    fn window(&self, from_qpc: i64, to_qpc: i64, modules: &[(u64, String)], lost: u64) -> DpcTrace {
        if self.keep_ticks.is_none() {
            return self.summarize(&self.routines, self.long.iter(), modules, lost, self.events);
        }
        let freq = self.freq.max(1);
        let (first, last) = (from_qpc.div_euclid(freq), to_qpc.div_euclid(freq));
        let mut routines: Routines = HashMap::new();
        let mut events = 0;
        for (_, bucket) in self
            .buckets
            .iter()
            .filter(|(s, _)| (first..=last).contains(s))
        {
            for (routine, st) in bucket {
                events += st.dpc + st.isr;
                let a = routines.entry(*routine).or_default();
                a.dpc += st.dpc;
                a.isr += st.isr;
                a.total_ticks += st.total_ticks;
                a.max_ticks = a.max_ticks.max(st.max_ticks);
            }
        }
        let long = self
            .long
            .iter()
            .filter(|l| l.0 >= from_qpc && l.0 <= to_qpc);
        self.summarize(&routines, long, modules, lost, events)
    }

    /// Підсумок із приписуванням процедур модулям `(база, ім'я)`.
    fn finish(self, modules: &[(u64, String)], lost: u64) -> DpcTrace {
        self.window(i64::MIN, i64::MAX, modules, lost)
    }

    fn summarize<'a>(
        &self,
        routines: &Routines,
        long: impl Iterator<Item = &'a RawLong>,
        modules: &[(u64, String)],
        lost: u64,
        events: u64,
    ) -> DpcTrace {
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
        for (routine, s) in routines {
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

        let mut long: Vec<LongDpc> = long
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
            events,
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

    /// Запускає сесію в кільцевому режимі: зберігає лише останні `keep_secs` секунд, тож може
    /// працювати годинами; зріз на льоту — [`DpcTracer::window`].
    pub fn start_ring(freq: i64, keep_secs: u32) -> Result<Self, EtwError> {
        #[cfg(windows)]
        {
            windows::Session::start(Collector::ring(freq, keep_secs)).map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = (freq, keep_secs);
            Err(EtwError::Unsupported)
        }
    }

    /// Зріз за `[from_qpc, to_qpc]` без зупинки сесії (у звичайному режимі — усе з початку
    /// сесії). `None`, якщо сесія вже не працює.
    pub fn window(&self, from_qpc: i64, to_qpc: i64) -> Option<DpcTrace> {
        #[cfg(windows)]
        {
            let lost = self.inner.lost_so_far();
            let modules = windows::kernel_modules();
            self.inner
                .with_collector(|c| c.window(from_qpc, to_qpc, &modules, lost))
        }
        #[cfg(not(windows))]
        {
            let _ = (from_qpc, to_qpc);
            None
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

    #[test]
    fn ring_keeps_only_recent_seconds_and_slices_windows() {
        // 1000 тиків = 1 с; тримаємо 3 с.
        let mut c = Collector::ring(1000, 3);
        let modules = vec![(0x1000, "a.sys".to_string()), (0x2000, "b.sys".to_string())];
        // Секунди 0..10: щосекунди коротка подія a.sys і довга (200 мс) b.sys.
        for sec in 0..10 {
            c.push(ev(sec * 1000 + 10, 0, 0x1001, DpcKind::Dpc)); // коротка (0 мс)
            c.push(ev(sec * 1000 + 20, 200, 0x2001, DpcKind::Isr));
        }
        // Старі секунди відкинуто: кошиків не більше вікна (+ поточний).
        assert!(c.buckets.len() <= 4, "{}", c.buckets.len());
        assert!(c.long.iter().all(|l| l.0 >= 6000), "{:?}", c.long);

        // Зріз за секунди 8–9.
        let t = c.window(8000, 9999, &modules, 0);
        assert_eq!(t.events, 4);
        assert_eq!(t.long.len(), 2);
        assert_eq!(t.long[0].start_qpc, 8020);
        let b = t
            .drivers
            .iter()
            .find(|d| d.name.as_deref() == Some("b.sys"))
            .unwrap();
        assert_eq!(b.isr_count, 2);
        assert!((b.max_us - 200_000.0).abs() < 1e-6);

        // Зріз поза збереженим — порожній.
        let old = c.window(0, 3000, &modules, 0);
        assert_eq!(old.events, 0);
        assert!(old.long.is_empty());

        // Подія із запізненням (інше ядро) потрапляє у свою секунду, якщо та ще є.
        c.push(ev(8500, 0, 0x1001, DpcKind::Dpc));
        let t = c.window(8000, 8999, &modules, 0);
        assert_eq!(t.events, 3);
    }

    #[test]
    fn normal_mode_ignores_window() {
        let mut c = Collector::new(1000);
        for sec in 0..10 {
            c.push(ev(sec * 1000, 1, 0x1001, DpcKind::Dpc));
        }
        let modules = vec![(0x1000, "a.sys".to_string())];
        assert_eq!(c.window(5000, 6000, &modules, 0).events, 10);
        assert_eq!(c.finish(&modules, 0).events, 10);
    }
}
