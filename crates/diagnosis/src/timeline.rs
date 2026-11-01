//! Спільна шкала часу: кадри гри та знімки метрик у мілісекундах від старту запису.

use optimax_core::clock::ticks_to_ms;
use optimax_core::etw::{DpcKind, DriverDpc};
use optimax_frametime::{main_swap_chain, PresentMode, Recording};
use optimax_metrics::{Snapshot, ThrottleReason};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Кадр на шкалі часу.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FramePoint {
    /// Час Present(), мс від старту.
    pub t_ms: f64,
    /// Frametime, мс.
    pub frametime_ms: f64,
    /// Час роботи CPU над кадром, мс.
    pub cpu_busy_ms: Option<f64>,
    /// Час роботи GPU над кадром, мс.
    pub gpu_busy_ms: Option<f64>,
    /// Шлях кадру на екран (колонка `PresentMode`).
    pub present_mode: PresentMode,
    /// Синхроінтервал, який запросила гра.
    pub sync_interval: Option<i32>,
    /// Чи дозволено розриви кадру.
    pub allows_tearing: Option<bool>,
    /// Скільки попередній кадр був на екрані, мс (`MsBetweenDisplayChange`; `None` — кадр
    /// не показано). Для генерації кадрів це і є те, що бачить око.
    #[serde(default)]
    pub display_ms: Option<f64>,
}

/// Стан відеокарти в момент знімка.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GpuPoint {
    /// Завантаження, %.
    pub util: Option<u32>,
    /// Частота ядра, МГц.
    pub clock_mhz: Option<u32>,
    /// Максимальна частота ядра, МГц.
    pub max_clock_mhz: Option<u32>,
    /// Зайнято VRAM, байт.
    pub vram_used: Option<u64>,
    /// Усього VRAM, байт.
    pub vram_total: Option<u64>,
    /// Температура, °C.
    pub temperature_c: Option<u32>,
    /// Причини зниження частоти.
    pub throttle: Vec<ThrottleReason>,
    /// Оберти вентиляторів, %.
    #[serde(default)]
    pub fan_pct: Option<u32>,
    /// Діючий ліміт потужності, Вт.
    #[serde(default)]
    pub power_limit_w: Option<f32>,
    /// Заводський ліміт потужності, Вт.
    #[serde(default)]
    pub default_power_limit_w: Option<f32>,
}

impl GpuPoint {
    /// Частка зайнятої VRAM (0–1).
    pub fn vram_fraction(&self) -> Option<f64> {
        let (u, t) = (self.vram_used?, self.vram_total?);
        (t > 0).then(|| u as f64 / t as f64)
    }
}

/// Знімок метрик на шкалі часу.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SamplePoint {
    /// Час, мс від старту.
    pub t_ms: f64,
    /// Індекс найзавантаженішого ядра.
    pub busiest_core: usize,
    /// Його завантаження, %.
    pub busiest_core_pct: f32,
    /// Середнє завантаження CPU, %.
    pub avg_cpu_pct: f32,
    /// Відеокарта, на якій працює гра (з найбільшим середнім завантаженням).
    pub gpu: Option<GpuPoint>,
    /// Читання + запис дисків, байт/с.
    pub disk_bytes_per_sec: u64,
    /// CPU сторонніх процесів за іменем (сума по всіх процесах з однаковим ім'ям), % усього CPU.
    pub background: HashMap<String, f32>,
    /// Частка вільної RAM (0–1).
    #[serde(default)]
    pub mem_available_fraction: Option<f64>,
    /// Сторінки, прочитані з диска через жорсткі page faults, за секунду.
    #[serde(default)]
    pub hard_faults_per_sec: Option<f64>,
    /// Час на DPC і переривання драйверів на найзайнятішому ядрі, %.
    #[serde(default)]
    pub dpc_pct: Option<f64>,
    /// Процес із найбільшим вводом-виводом (зокрема сама гра).
    #[serde(default)]
    pub io_top: Option<IoProcess>,
}

/// Процес, що найбільше читав і писав у момент знімка.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IoProcess {
    /// Ім'я exe.
    pub name: String,
    /// Читання + запис, байт/с.
    pub bytes_per_sec: u64,
}

/// Файл, записаний у кеш шейдерів під час запису (час останньої зміни на шкалі запису).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShaderWrite {
    /// Час зміни файлу, мс від старту запису.
    pub t_ms: f64,
    /// Розмір файлу, байт.
    pub bytes: u64,
}

/// Довга подія DPC/ISR драйвера на шкалі часу.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DpcPoint {
    /// Початок виконання, мс від старту запису.
    pub t_ms: f64,
    /// Тривалість, мкс.
    pub dur_us: f64,
    /// Індекс у [`DpcTimeline::drivers`].
    pub driver: u16,
    /// Обробник переривання (а не DPC).
    pub isr: bool,
}

/// Трасування драйверів за час запису.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DpcTimeline {
    /// Підсумок по драйверах (за весь запис, зокрема поза зарахованими відрізками).
    pub drivers: Vec<DriverDpc>,
    /// Довгі події в зарахованих відрізках, за часом.
    pub long: Vec<DpcPoint>,
    /// Подій, втрачених ETW.
    pub lost: u64,
}

impl DpcTimeline {
    /// Ім'я драйвера події (`None` — адреса поза відомими модулями).
    pub fn driver_name(&self, e: &DpcPoint) -> Option<&str> {
        self.drivers.get(usize::from(e.driver))?.name.as_deref()
    }

    /// Довгі події, що почались у `[from, to]` (мс).
    pub fn between(&self, from: f64, to: f64) -> &[DpcPoint] {
        let lo = self.long.partition_point(|e| e.t_ms < from);
        let hi = self.long.partition_point(|e| e.t_ms <= to);
        &self.long[lo..hi.max(lo)]
    }
}

/// Дані для діагностики.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    /// PID гри.
    pub game_pid: u32,
    /// Ім'я процесу гри.
    pub game_name: String,
    /// Кадри головного swap chain гри, відсортовані за часом.
    pub frames: Vec<FramePoint>,
    /// Знімки метрик, відсортовані за часом.
    pub samples: Vec<SamplePoint>,
    /// Записи в кеш шейдерів драйвера під час запису, відсортовані за часом.
    #[serde(default)]
    pub shader_writes: Vec<ShaderWrite>,
    /// DPC і переривання драйверів (ETW). `None` — не вимірювалось.
    #[serde(default)]
    pub dpc: Option<DpcTimeline>,
}

/// Процеси, які не вважаються фоновими навантаженнями (наш власний інструмент запису).
fn is_own_tool(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("presentmon")
}

fn primary_gpu_index(snapshots: &[Snapshot]) -> Option<u32> {
    let mut sum: HashMap<u32, (u64, u64)> = HashMap::new();
    for s in snapshots {
        for g in &s.gpus {
            let e = sum.entry(g.index).or_default();
            e.0 += u64::from(g.utilization_percent.unwrap_or(0));
            e.1 += 1;
        }
    }
    sum.into_iter()
        .max_by(|a, b| {
            let avg = |(s, n): (u64, u64)| s as f64 / n.max(1) as f64;
            avg(a.1).total_cmp(&avg(b.1)).then(b.0.cmp(&a.0))
        })
        .map(|(i, _)| i)
}

impl Timeline {
    /// Будує шкалу з запису. `None`, якщо кадрів немає.
    /// Береться лише зарахована гра (відрізки [`Recording::segments`]): Alt+Tab, меню
    /// й завантаження, що потрапили в запис, на результат не впливають.
    pub fn from_recording(rec: &Recording) -> Option<Self> {
        let counted: Vec<optimax_frametime::FrameRecord> = rec
            .frames
            .iter()
            .filter(|f| rec.frame_counted(f))
            .cloned()
            .collect();
        let (pid, swap_chain) = main_swap_chain(&counted)?;
        let game_name = counted
            .iter()
            .find(|f| f.process_id == pid)
            .map(|f| f.application.clone())
            .unwrap_or_default();
        // PresentMon видає кадри не за порядком часу — сортуємо.
        let mut frames: Vec<FramePoint> = counted
            .iter()
            .filter(|f| f.process_id == pid && f.swap_chain == swap_chain)
            .map(|f| FramePoint {
                t_ms: rec.frame_ms(f),
                frametime_ms: f.ms_between_presents,
                cpu_busy_ms: f.ms_cpu_busy,
                gpu_busy_ms: f.ms_gpu_busy,
                present_mode: PresentMode::parse(&f.present_mode),
                sync_interval: f.sync_interval,
                allows_tearing: f.allows_tearing,
                display_ms: f.ms_between_display_change,
            })
            .collect();
        frames.sort_by(|a, b| a.t_ms.total_cmp(&b.t_ms));

        let gpu_index = primary_gpu_index(&rec.snapshots);
        let mut samples: Vec<SamplePoint> = rec
            .snapshots
            .iter()
            .filter(|s| rec.in_segments(s.qpc))
            .map(|s| {
                let (busiest_core, busiest_core_pct) = s.cpu.busiest_core().unwrap_or((0, 0.0));
                let mut background: HashMap<String, f32> = HashMap::new();
                for p in &s.top_processes {
                    if p.pid != pid && !is_own_tool(&p.name) {
                        *background.entry(p.name.clone()).or_default() += p.cpu_percent;
                    }
                }
                SamplePoint {
                    t_ms: rec.snapshot_ms(s),
                    busiest_core,
                    busiest_core_pct,
                    avg_cpu_pct: s.cpu.total_percent,
                    gpu: gpu_index
                        .and_then(|i| s.gpus.iter().find(|g| g.index == i))
                        .map(|g| GpuPoint {
                            util: g.utilization_percent,
                            clock_mhz: g.core_clock_mhz,
                            max_clock_mhz: g.max_core_clock_mhz,
                            vram_used: g.vram_used_bytes,
                            vram_total: g.vram_total_bytes,
                            temperature_c: g.temperature_c,
                            throttle: g.throttle.clone(),
                            fan_pct: g.fan_percent,
                            power_limit_w: g.power_limit_w,
                            default_power_limit_w: g.default_power_limit_w,
                        }),
                    disk_bytes_per_sec: s
                        .disk
                        .read_bytes_per_sec
                        .saturating_add(s.disk.write_bytes_per_sec),
                    background,
                    mem_available_fraction: (s.memory.total_bytes > 0)
                        .then(|| s.memory.available_bytes as f64 / s.memory.total_bytes as f64),
                    hard_faults_per_sec: s.memory.hard_fault_pages_per_sec,
                    dpc_pct: s.cpu.dpc_percent.map(f64::from),
                    io_top: s
                        .top_processes
                        .iter()
                        .filter(|p| p.io_bytes_per_sec > 0 && !is_own_tool(&p.name))
                        .max_by_key(|p| p.io_bytes_per_sec)
                        .map(|p| IoProcess {
                            name: p.name.clone(),
                            bytes_per_sec: p.io_bytes_per_sec,
                        }),
                }
            })
            .collect();
        samples.sort_by(|a, b| a.t_ms.total_cmp(&b.t_ms));
        Some(Self {
            game_pid: pid,
            game_name,
            frames,
            samples,
            shader_writes: Vec::new(),
            dpc: rec.dpc_trace.as_ref().map(|t| DpcTimeline {
                drivers: t.drivers.clone(),
                long: t
                    .long
                    .iter()
                    .filter(|e| rec.in_segments(e.start_qpc))
                    .map(|e| DpcPoint {
                        t_ms: ticks_to_ms(e.start_qpc - rec.start_qpc, rec.qpc_frequency),
                        dur_us: f64::from(e.dur_us),
                        driver: e.driver,
                        isr: e.kind == DpcKind::Isr,
                    })
                    .collect(),
                lost: t.lost,
            }),
        })
    }

    /// Знімки у вікні `[t - radius, t + radius]`.
    pub fn samples_near(&self, t_ms: f64, radius_ms: f64) -> &[SamplePoint] {
        let lo = self.samples.partition_point(|s| s.t_ms < t_ms - radius_ms);
        let hi = self.samples.partition_point(|s| s.t_ms <= t_ms + radius_ms);
        &self.samples[lo..hi.max(lo)]
    }

    /// Тривалість запису кадрів, мс.
    pub fn duration_ms(&self) -> f64 {
        self.frames.iter().map(|f| f.frametime_ms).sum()
    }
}
