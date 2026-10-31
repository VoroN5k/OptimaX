//! Джерело CPU/RAM/диск/процеси на базі `sysinfo`.

use crate::snapshot::{
    normalize_process_cpu, per_second, top_with_io, CpuSnapshot, DiskSnapshot, MemorySnapshot,
    ProcessSample,
};
use std::time::{Duration, Instant};
use sysinfo::{
    CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, ProcessRefreshKind,
    ProcessesToUpdate, RefreshKind, System,
};

/// Джерело системних метрик (CPU, RAM, диск, процеси).
pub trait SystemSource: Send {
    /// Завантаження та частоти CPU з моменту попереднього виклику.
    fn cpu(&mut self) -> CpuSnapshot;
    /// Стан RAM.
    fn memory(&mut self) -> MemorySnapshot;
    /// Активність дисків за `elapsed` з попереднього виклику.
    fn disk(&mut self, elapsed: Duration) -> DiskSnapshot;
    /// Топ `n` процесів за CPU з моменту попереднього виклику.
    fn top_processes(&mut self, n: usize) -> Vec<ProcessSample>;
}

/// Реалізація [`SystemSource`] через `sysinfo`. Оновлює лише потрібні поля, щоб не навантажувати систему.
pub struct SysinfoSource {
    system: System,
    disks: Disks,
    own_pid: Option<u32>,
    hard_faults: Option<optimax_core::perf::HardFaultCounter>,
    /// Коли востаннє оновлювали процеси (для вводу-виводу за секунду).
    processes_at: Option<Instant>,
}

impl SysinfoSource {
    /// Створює джерело й робить перший замір (потрібен для розрахунку дельт).
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage().with_frequency())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        let disks =
            Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_io_usage());
        Self {
            system,
            disks,
            own_pid: sysinfo::get_current_pid().ok().map(|p| p.as_u32()),
            hard_faults: optimax_core::perf::HardFaultCounter::open(),
            processes_at: None,
        }
    }
}

impl Default for SysinfoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemSource for SysinfoSource {
    fn cpu(&mut self) -> CpuSnapshot {
        self.system
            .refresh_cpu_specifics(CpuRefreshKind::nothing().with_cpu_usage().with_frequency());
        let cpus = self.system.cpus();
        CpuSnapshot {
            total_percent: self.system.global_cpu_usage(),
            per_core_percent: cpus.iter().map(|c| c.cpu_usage()).collect(),
            per_core_mhz: cpus.iter().map(|c| c.frequency()).collect(),
        }
    }

    fn memory(&mut self) -> MemorySnapshot {
        self.system
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        MemorySnapshot {
            total_bytes: self.system.total_memory(),
            used_bytes: self.system.used_memory(),
            available_bytes: self.system.available_memory(),
            hard_fault_pages_per_sec: self.hard_faults.as_mut().and_then(|c| c.sample()),
        }
    }

    fn disk(&mut self, elapsed: Duration) -> DiskSnapshot {
        self.disks
            .refresh_specifics(true, DiskRefreshKind::nothing().with_io_usage());
        let (read, written) = self.disks.list().iter().fold((0u64, 0u64), |(r, w), d| {
            let u = d.usage();
            (
                r.saturating_add(u.read_bytes),
                w.saturating_add(u.written_bytes),
            )
        });
        DiskSnapshot {
            read_bytes_per_sec: per_second(read, elapsed),
            write_bytes_per_sec: per_second(written, elapsed),
        }
    }

    fn top_processes(&mut self, n: usize) -> Vec<ProcessSample> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage(),
        );
        let now = Instant::now();
        // Перше оновлення дає ввід-вивід від старту процесу, а не за інтервал — його не беремо.
        let elapsed = self.processes_at.map(|t| now.duration_since(t));
        self.processes_at = Some(now);
        let logical = self.system.cpus().len();
        let samples = self
            .system
            .processes()
            .values()
            // PID 0 — «Бездіяльність системи», це не навантаження.
            .filter(|p| p.pid().as_u32() != 0 && Some(p.pid().as_u32()) != self.own_pid)
            .map(|p| ProcessSample {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: normalize_process_cpu(p.cpu_usage(), logical),
                memory_bytes: p.memory(),
                io_bytes_per_sec: elapsed.map_or(0, |e| {
                    let u = p.disk_usage();
                    per_second(u.read_bytes.saturating_add(u.written_bytes), e)
                }),
            })
            .collect();
        top_with_io(samples, n)
    }
}
