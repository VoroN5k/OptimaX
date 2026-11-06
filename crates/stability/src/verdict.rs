//! Вердикт, запобіжники й запис результату (чисті функції над даними).

use serde::{Deserialize, Serialize};

/// Температура GPU, на якій тест зупиняється сам, °C (рішення користувача, 2026-11-06).
pub const GPU_STOP_TEMP_C: u32 = 85;
/// Скільки може не бути показань температури, якщо датчик був, с.
pub const SENSOR_LOST_S: f64 = 10.0;
/// Середнє завантаження GPU, нижче якого навантаження було неповним, %. Припущення.
pub const LOW_LOAD_PCT: f64 = 80.0;
/// Дозволені тривалості тесту, с: швидка перевірка, 10 хв, 30 хв.
pub const DURATIONS_S: [u32; 3] = [120, 600, 1800];

/// Чому запобіжник зупинив тест.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardStop {
    /// Температура досягла межі.
    OverTemp,
    /// Датчик температури перестав відповідати.
    SensorLost,
}

/// Запобіжник температури.
#[derive(Debug, Clone)]
pub struct Guard {
    enabled: bool,
    last_reading_s: f64,
}

impl Guard {
    /// `has_sensor` — на старті температура була доступна (інакше стежити нема за чим, і запустити
    /// тест можна лише після явного попередження).
    pub fn new(has_sensor: bool) -> Self {
        Self {
            enabled: has_sensor,
            last_reading_s: 0.0,
        }
    }

    /// Нове показання в момент `now_s`.
    pub fn check(&mut self, temp_c: Option<u32>, now_s: f64) -> Option<GuardStop> {
        if !self.enabled {
            return None;
        }
        match temp_c {
            Some(t) => {
                self.last_reading_s = now_s;
                (t >= GPU_STOP_TEMP_C).then_some(GuardStop::OverTemp)
            }
            None => (now_s - self.last_reading_s > SENSOR_LOST_S).then_some(GuardStop::SensorLost),
        }
    }
}

/// Як закінчився процес навантаження.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StabEnd {
    /// Час вийшов.
    Completed,
    /// Зупинив користувач.
    Cancelled,
    /// Зупинено через температуру.
    OverTemp,
    /// Зупинено, бо зник датчик температури.
    SensorLost,
    /// Драйвер скинув відеокарту або вона зникла.
    DeviceLost,
    /// Процес упав або завис.
    WorkerDied,
    /// Тест не вдалося виконати.
    Failed,
    /// Програма чи ПК зникли посеред тесту (знайдено позначку після запуску).
    Interrupted,
}

/// Події журналу Windows під час тесту.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StabEvents {
    /// Журнал вдалося прочитати.
    pub read: bool,
    /// Скидання драйвера відеокарти (TDR).
    pub tdr: u32,
    /// Виправлені апаратні помилки WHEA.
    pub whea_corrected: u32,
    /// Невиправлені апаратні помилки WHEA.
    pub whea_fatal: u32,
    /// Раптові вимкнення або сині екрани.
    pub unexpected_off: u32,
}

/// Підсумок тесту.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StabOutcome {
    /// Тест пройшов до кінця, помилок не знайдено.
    NoErrors,
    /// Результати обчислень розійшлися.
    Errors,
    /// Драйвер скинув відеокарту.
    DeviceLost,
    /// Обчислення збіглися, але Windows записала апаратні помилки.
    HardwareEvents,
    /// Процес навантаження впав або завис.
    WorkerCrashed,
    /// Тест перервано вимкненням ПК або закриттям програми.
    Interrupted,
    /// Зупинено через температуру: висновку про стабільність немає.
    OverTemp,
    /// Зупинено, бо зник датчик температури.
    SensorLost,
    /// Жоден повтор не встиг звіритись.
    TooShort,
    /// Зупинив користувач, помилок до зупинки не знайдено.
    Cancelled,
    /// Тест не вдалося виконати (не висновок про залізо).
    Failed,
}

/// Підсумок. Знайдена помилка важливіша за спосіб завершення: вона лишається фактом, навіть якщо
/// тест потім зупинили чи він перегрівся.
pub fn outcome(end: StabEnd, mismatches: u64, checked: u64, events: &StabEvents) -> StabOutcome {
    if mismatches > 0 {
        return StabOutcome::Errors;
    }
    if end == StabEnd::DeviceLost || events.tdr > 0 {
        return StabOutcome::DeviceLost;
    }
    match end {
        StabEnd::WorkerDied => StabOutcome::WorkerCrashed,
        StabEnd::Interrupted => StabOutcome::Interrupted,
        StabEnd::OverTemp => StabOutcome::OverTemp,
        StabEnd::SensorLost => StabOutcome::SensorLost,
        StabEnd::Failed => StabOutcome::Failed,
        StabEnd::Cancelled => StabOutcome::Cancelled,
        StabEnd::DeviceLost | StabEnd::Completed => {
            if events.whea_fatal > 0 || events.whea_corrected > 0 || events.unexpected_off > 0 {
                StabOutcome::HardwareEvents
            } else if checked == 0 {
                StabOutcome::TooShort
            } else {
                StabOutcome::NoErrors
            }
        }
    }
}

/// Показання під час тесту.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StabStats {
    /// Найвища температура GPU, °C.
    pub max_temp_c: Option<u32>,
    /// Середня частота ядра, МГц.
    pub avg_clock_mhz: Option<f64>,
    /// Середнє споживання, Вт.
    pub avg_power_w: Option<f64>,
    /// Середнє завантаження, %.
    pub avg_util_pct: Option<f64>,
}

/// Накопичує показання (перші секунди, поки навантаження запускається, теж ідуть у середнє, тож
/// при коротких тестах воно трохи занижене).
#[derive(Debug, Clone, Default)]
pub struct StatsAcc {
    max_temp: Option<u32>,
    clock: (f64, u32),
    power: (f64, u32),
    util: (f64, u32),
}

impl StatsAcc {
    /// Одне показання.
    pub fn add(
        &mut self,
        temp: Option<u32>,
        clock: Option<u32>,
        power: Option<f32>,
        util: Option<u32>,
    ) {
        fn push(slot: &mut (f64, u32), v: Option<f64>) {
            if let Some(v) = v {
                slot.0 += v;
                slot.1 += 1;
            }
        }
        self.max_temp = self.max_temp.max(temp);
        push(&mut self.clock, clock.map(f64::from));
        push(&mut self.power, power.map(f64::from));
        push(&mut self.util, util.map(f64::from));
    }

    /// Підсумок.
    pub fn stats(&self) -> StabStats {
        let avg = |s: (f64, u32)| (s.1 > 0).then(|| s.0 / f64::from(s.1));
        StabStats {
            max_temp_c: self.max_temp,
            avg_clock_mhz: avg(self.clock),
            avg_power_w: avg(self.power),
            avg_util_pct: avg(self.util),
        }
    }
}

/// Результат тесту (зберігається в теці даних).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StabRecord {
    /// Ідентифікатор (ім'я файлу).
    pub id: String,
    /// Початок, Unix-мс.
    pub started_unix_ms: i64,
    /// Назва відеокарти.
    pub adapter: String,
    /// Запланована тривалість, с.
    pub planned_s: u32,
    /// Фактична тривалість, с.
    pub elapsed_s: f64,
    /// Завдань виконано.
    pub rounds: u64,
    /// Повторів звірено.
    pub checked: u64,
    /// Розбіжностей.
    pub mismatches: u64,
    /// Скільки відеопам'яті перевірено, МБ.
    pub tested_mb: u64,
    /// Обсяг відеопам'яті, МБ.
    pub vram_mb: u64,
    /// Підсумок.
    pub outcome: StabOutcome,
    /// Події журналу Windows під час тесту.
    pub events: StabEvents,
    /// Показання.
    pub stats: StabStats,
    /// Навантаження було неповним (див. [`LOW_LOAD_PCT`]).
    pub low_load: bool,
    /// Запущено без датчика температури (після попередження).
    pub no_sensor: bool,
}

/// Хід тесту, що йде. Цей самий стан лежить на диску, поки тест іде: якщо ПК перезавантажиться,
/// після запуску програма побачить незавершений тест.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StabView {
    /// Ідентифікатор.
    pub id: String,
    /// Початок, Unix-мс.
    pub started_unix_ms: i64,
    /// Назва відеокарти (порожня, поки пристрій не створено).
    pub adapter: String,
    /// Запланована тривалість, с.
    pub planned_s: u32,
    /// Минуло, с.
    pub elapsed_s: f64,
    /// Завдань виконано.
    pub rounds: u64,
    /// Повторів звірено.
    pub checked: u64,
    /// Розбіжностей.
    pub mismatches: u64,
    /// Скільки відеопам'яті перевіряється, МБ.
    pub tested_mb: u64,
    /// Обсяг відеопам'яті, МБ.
    pub vram_mb: u64,
    /// Температура GPU, °C.
    pub temp_c: Option<u32>,
    /// Частота ядра, МГц.
    pub clock_mhz: Option<u32>,
    /// Споживання, Вт.
    pub power_w: Option<f32>,
    /// Завантаження, %.
    pub util_pct: Option<u32>,
    /// Межа температури, °C (`None` — датчика немає).
    pub limit_c: Option<u32>,
    /// Показання, накопичені до цього моменту.
    pub stats: StabStats,
    /// Запущено без датчика температури.
    pub no_sensor: bool,
}

impl StabView {
    /// Запис результату за цим станом.
    pub fn into_record(self, end: StabEnd, events: StabEvents) -> StabRecord {
        let low_load = self.stats.avg_util_pct.is_some_and(|u| u < LOW_LOAD_PCT);
        StabRecord {
            outcome: outcome(end, self.mismatches, self.checked, &events),
            id: self.id,
            started_unix_ms: self.started_unix_ms,
            adapter: self.adapter,
            planned_s: self.planned_s,
            elapsed_s: self.elapsed_s,
            rounds: self.rounds,
            checked: self.checked,
            mismatches: self.mismatches,
            tested_mb: self.tested_mb,
            vram_mb: self.vram_mb,
            events,
            stats: self.stats,
            low_load,
            no_sensor: self.no_sensor,
        }
    }
}

/// Що відомо до запуску.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StabInfo {
    /// Відеокарта за NVML (`None` — не NVIDIA, назву покаже сам тест).
    pub gpu: Option<String>,
    /// Температура доступна.
    pub has_sensor: bool,
    /// Температура зупинки, °C.
    pub limit_c: u32,
    /// Дозволені тривалості, с.
    pub durations_s: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: StabEvents = StabEvents {
        read: true,
        tdr: 0,
        whea_corrected: 0,
        whea_fatal: 0,
        unexpected_off: 0,
    };

    #[test]
    fn clean_completed_run_finds_no_errors() {
        assert_eq!(
            outcome(StabEnd::Completed, 0, 10, &CLEAN),
            StabOutcome::NoErrors
        );
    }

    #[test]
    fn nothing_compared_means_no_conclusion() {
        assert_eq!(
            outcome(StabEnd::Completed, 0, 0, &CLEAN),
            StabOutcome::TooShort
        );
    }

    #[test]
    fn a_found_error_stays_a_fact_however_the_test_ended() {
        for end in [
            StabEnd::Completed,
            StabEnd::Cancelled,
            StabEnd::OverTemp,
            StabEnd::DeviceLost,
            StabEnd::WorkerDied,
            StabEnd::Interrupted,
        ] {
            assert_eq!(outcome(end, 1, 5, &CLEAN), StabOutcome::Errors, "{end:?}");
        }
    }

    #[test]
    fn driver_reset_counts_from_the_worker_or_the_event_log() {
        assert_eq!(
            outcome(StabEnd::DeviceLost, 0, 5, &CLEAN),
            StabOutcome::DeviceLost
        );
        let tdr = StabEvents { tdr: 1, ..CLEAN };
        assert_eq!(
            outcome(StabEnd::Completed, 0, 5, &tdr),
            StabOutcome::DeviceLost
        );
        assert_eq!(
            outcome(StabEnd::WorkerDied, 0, 5, &tdr),
            StabOutcome::DeviceLost
        );
    }

    #[test]
    fn stopped_early_is_not_a_clean_result() {
        assert_eq!(
            outcome(StabEnd::OverTemp, 0, 5, &CLEAN),
            StabOutcome::OverTemp
        );
        assert_eq!(
            outcome(StabEnd::SensorLost, 0, 5, &CLEAN),
            StabOutcome::SensorLost
        );
        assert_eq!(
            outcome(StabEnd::Cancelled, 0, 5, &CLEAN),
            StabOutcome::Cancelled
        );
        assert_eq!(
            outcome(StabEnd::WorkerDied, 0, 5, &CLEAN),
            StabOutcome::WorkerCrashed
        );
        assert_eq!(
            outcome(StabEnd::Interrupted, 0, 5, &CLEAN),
            StabOutcome::Interrupted
        );
        assert_eq!(outcome(StabEnd::Failed, 0, 5, &CLEAN), StabOutcome::Failed);
    }

    #[test]
    fn hardware_errors_in_the_log_spoil_a_clean_run() {
        for events in [
            StabEvents {
                whea_corrected: 3,
                ..CLEAN
            },
            StabEvents {
                whea_fatal: 1,
                ..CLEAN
            },
            StabEvents {
                unexpected_off: 1,
                ..CLEAN
            },
        ] {
            assert_eq!(
                outcome(StabEnd::Completed, 0, 5, &events),
                StabOutcome::HardwareEvents
            );
        }
    }

    #[test]
    fn guard_stops_at_the_limit_and_when_the_sensor_disappears() {
        let mut g = Guard::new(true);
        assert_eq!(g.check(Some(GPU_STOP_TEMP_C - 1), 1.0), None);
        assert_eq!(g.check(None, 1.0 + SENSOR_LOST_S), None);
        assert_eq!(
            g.check(None, 1.1 + SENSOR_LOST_S),
            Some(GuardStop::SensorLost)
        );
        assert_eq!(
            g.check(Some(GPU_STOP_TEMP_C), 20.0),
            Some(GuardStop::OverTemp)
        );
    }

    #[test]
    fn guard_without_a_sensor_watches_nothing() {
        let mut g = Guard::new(false);
        assert_eq!(g.check(None, 1000.0), None);
        assert_eq!(g.check(Some(200), 1000.0), None);
    }

    #[test]
    fn stats_skip_missing_readings() {
        let mut a = StatsAcc::default();
        assert_eq!(a.stats(), StabStats::default());
        a.add(Some(60), Some(2000), None, Some(90));
        a.add(Some(70), None, Some(200.0), Some(100));
        a.add(None, Some(2200), Some(220.0), None);
        let s = a.stats();
        assert_eq!(s.max_temp_c, Some(70));
        assert_eq!(s.avg_clock_mhz, Some(2100.0));
        assert_eq!(s.avg_power_w, Some(210.0));
        assert_eq!(s.avg_util_pct, Some(95.0));
    }

    #[test]
    fn low_load_is_flagged_on_the_record() {
        let view = |util| StabView {
            id: "1-gpu".into(),
            started_unix_ms: 1,
            adapter: "GPU".into(),
            planned_s: 120,
            elapsed_s: 120.0,
            rounds: 40,
            checked: 36,
            mismatches: 0,
            tested_mb: 2048,
            vram_mb: 12288,
            temp_c: None,
            clock_mhz: None,
            power_w: None,
            util_pct: None,
            limit_c: Some(GPU_STOP_TEMP_C),
            stats: StabStats {
                avg_util_pct: Some(util),
                ..StabStats::default()
            },
            no_sensor: false,
        };
        assert!(view(50.0).into_record(StabEnd::Completed, CLEAN).low_load);
        let r = view(99.0).into_record(StabEnd::Completed, CLEAN);
        assert!(!r.low_load);
        assert_eq!(r.outcome, StabOutcome::NoErrors);
    }
}
