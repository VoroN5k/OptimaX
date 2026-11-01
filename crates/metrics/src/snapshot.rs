//! Типи знімків метрик і чисті допоміжні функції.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Один знімок усіх метрик.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Snapshot {
    /// Мілісекунди від старту семплера (монотонний годинник).
    pub elapsed_ms: u64,
    /// Мітка QPC моменту знімка — та сама шкала, що й час кадрів PresentMon (`--qpc_time`).
    pub qpc: i64,
    /// Процесор.
    pub cpu: CpuSnapshot,
    /// Оперативна пам'ять.
    pub memory: MemorySnapshot,
    /// Диски (сума по всіх).
    pub disk: DiskSnapshot,
    /// Відеокарти, для яких є джерело даних.
    pub gpus: Vec<GpuSnapshot>,
    /// Топ процесів за CPU. Оновлюється рідше за решту (див. [`crate::SamplerConfig`]),
    /// у проміжних знімках містить останнє відоме значення.
    pub top_processes: Vec<ProcessSample>,
}

/// Завантаження CPU.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CpuSnapshot {
    /// Середнє по всіх логічних ядрах, %.
    pub total_percent: f32,
    /// Завантаження кожного логічного ядра, %.
    pub per_core_percent: Vec<f32>,
    /// Частота кожного ядра, МГц (на Windows часто показує базову, а не реальну частоту).
    pub per_core_mhz: Vec<u64>,
    /// Час на DPC і переривання драйверів на найзайнятішому ядрі, % (PDH). `None` —
    /// лічильник недоступний або це перший замір.
    #[serde(default)]
    pub dpc_percent: Option<f32>,
}

impl CpuSnapshot {
    /// Найзавантаженіше ядро: (індекс, %).
    pub fn busiest_core(&self) -> Option<(usize, f32)> {
        self.per_core_percent
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Стан RAM.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemorySnapshot {
    /// Усього, байт.
    pub total_bytes: u64,
    /// Використано, байт.
    pub used_bytes: u64,
    /// Доступно, байт.
    pub available_bytes: u64,
    /// Сторінки, прочитані з диска через жорсткі page faults, за секунду (PDH
    /// `\Memory\Pages Input/sec`). `None` — лічильник недоступний або це перший замір.
    #[serde(default)]
    pub hard_fault_pages_per_sec: Option<f64>,
}

/// Активність дисків.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DiskSnapshot {
    /// Читання, байт/с.
    pub read_bytes_per_sec: u64,
    /// Запис, байт/с.
    pub write_bytes_per_sec: u64,
}

/// Причина зниження частоти GPU (NVML `ClocksThrottleReasons`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ThrottleReason {
    /// GPU простоює.
    Idle,
    /// Частоти зафіксовані налаштуванням застосунку або користувачем.
    ApplicationClocks,
    /// Програмний ліміт потужності (power limit).
    SwPowerCap,
    /// Апаратне сповільнення (перегрів або живлення).
    HwSlowdown,
    /// Синхронізація буста між GPU.
    SyncBoost,
    /// Програмний тепловий ліміт.
    SwThermal,
    /// Апаратний тепловий ліміт.
    HwThermal,
    /// Апаратне гальмо живлення (зовнішній сигнал БЖ).
    HwPowerBrake,
    /// Обмеження через налаштування дисплея.
    DisplayClock,
}

/// Метрики однієї відеокарти. Поля `None` — джерело не повернуло значення.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GpuSnapshot {
    /// Індекс у джерелі (NVML).
    pub index: u32,
    /// Назва моделі.
    pub name: String,
    /// Завантаження ядра GPU, %.
    pub utilization_percent: Option<u32>,
    /// Використано VRAM, байт.
    pub vram_used_bytes: Option<u64>,
    /// Усього VRAM, байт.
    pub vram_total_bytes: Option<u64>,
    /// Частота ядра, МГц.
    pub core_clock_mhz: Option<u32>,
    /// Максимальна частота ядра, яку підтримує карта, МГц.
    pub max_core_clock_mhz: Option<u32>,
    /// Частота пам'яті, МГц.
    pub memory_clock_mhz: Option<u32>,
    /// Температура ядра, °C.
    pub temperature_c: Option<u32>,
    /// Споживання, Вт.
    pub power_w: Option<f32>,
    /// Діючий ліміт потужності, Вт.
    pub power_limit_w: Option<f32>,
    /// Заводський ліміт потужності, Вт (вищий діючий — розгін, напр. Power Limit 115%).
    #[serde(default)]
    pub default_power_limit_w: Option<f32>,
    /// Оберти вентиляторів, % від максимуму (середнє по вентиляторах; цільове значення драйвера).
    #[serde(default)]
    pub fan_percent: Option<u32>,
    /// Активні причини зниження частоти.
    pub throttle: Vec<ThrottleReason>,
}

/// Процес у топі за CPU.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProcessSample {
    /// PID.
    pub pid: u32,
    /// Ім'я виконуваного файлу.
    pub name: String,
    /// Частка від **усього** CPU, % (0–100, а не «% одного ядра»).
    pub cpu_percent: f32,
    /// Робочий набір, байт.
    pub memory_bytes: u64,
    /// Читання + запис за секунду, байт (лічильники вводу-виводу процесу: файли, а також
    /// інші пристрої, зокрема мережа). 0 — ще не виміряно.
    #[serde(default)]
    pub io_bytes_per_sec: u64,
}

/// Переводить «% одного ядра» (як віддає `sysinfo`, до N×100) у частку від усього CPU.
pub fn normalize_process_cpu(raw_percent: f32, logical_cores: usize) -> f32 {
    if logical_cores == 0 || !raw_percent.is_finite() {
        return 0.0;
    }
    (raw_percent / logical_cores as f32).clamp(0.0, 100.0)
}

/// Залишає `n` процесів із найбільшим CPU, від більшого до меншого.
pub fn top_by_cpu(mut processes: Vec<ProcessSample>, n: usize) -> Vec<ProcessSample> {
    processes.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    processes.truncate(n);
    processes
}

/// Топ `n` за CPU, плюс процес із найбільшим вводом-виводом, якщо його там немає: щоб біля
/// ривка можна було назвати, хто навантажував диск, навіть коли за CPU він непомітний.
pub fn top_with_io(processes: Vec<ProcessSample>, n: usize) -> Vec<ProcessSample> {
    let busiest_io = processes
        .iter()
        .filter(|p| p.io_bytes_per_sec > 0)
        .max_by_key(|p| p.io_bytes_per_sec)
        .cloned();
    let mut top = top_by_cpu(processes, n);
    if let Some(io) = busiest_io {
        if !top.iter().any(|p| p.pid == io.pid) {
            top.push(io);
        }
    }
    top
}

/// Байти за інтервал → байти за секунду.
pub fn per_second(bytes: u64, elapsed: Duration) -> u64 {
    let ms = elapsed.as_millis();
    if ms == 0 {
        return 0;
    }
    u64::try_from(u128::from(bytes) * 1000 / ms).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, cpu: f32) -> ProcessSample {
        ProcessSample {
            pid,
            name: format!("p{pid}"),
            cpu_percent: cpu,
            memory_bytes: 0,
            io_bytes_per_sec: 0,
        }
    }

    #[test]
    fn busiest_io_process_is_kept_even_with_low_cpu() {
        let mut quiet_reader = proc(9, 0.1);
        quiet_reader.io_bytes_per_sec = 80 << 20;
        let list = vec![proc(1, 30.0), proc(2, 20.0), proc(3, 10.0), quiet_reader];
        let top = top_with_io(list, 2);
        let pids: Vec<u32> = top.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![1, 2, 9]);
        // Якщо він і так у топі за CPU — не дублюється.
        let mut busy = proc(1, 30.0);
        busy.io_bytes_per_sec = 1 << 20;
        let top = top_with_io(vec![busy, proc(2, 20.0)], 2);
        assert_eq!(top.len(), 2);
        // Без вводу-виводу — просто топ за CPU.
        assert_eq!(top_with_io(vec![proc(1, 1.0), proc(2, 2.0)], 1).len(), 1);
    }

    #[test]
    fn normalizes_per_core_percent() {
        assert_eq!(normalize_process_cpu(200.0, 16), 12.5);
        assert_eq!(normalize_process_cpu(5000.0, 16), 100.0);
        assert_eq!(normalize_process_cpu(50.0, 0), 0.0);
        assert_eq!(normalize_process_cpu(f32::NAN, 8), 0.0);
    }

    #[test]
    fn top_sorts_and_truncates() {
        let top = top_by_cpu(vec![proc(1, 1.0), proc(2, 13.0), proc(3, 5.0)], 2);
        assert_eq!(top.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn busiest_core_uses_each_core() {
        let cpu = CpuSnapshot {
            total_percent: 20.0,
            per_core_percent: vec![10.0, 98.0, 5.0],
            per_core_mhz: vec![],
            dpc_percent: None,
        };
        assert_eq!(cpu.busiest_core(), Some((1, 98.0)));
        assert_eq!(CpuSnapshot::default().busiest_core(), None);
    }

    #[test]
    fn rate_per_second() {
        assert_eq!(per_second(1000, Duration::from_millis(250)), 4000);
        assert_eq!(per_second(1000, Duration::ZERO), 0);
    }
}
