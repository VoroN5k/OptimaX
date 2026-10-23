//! Заміри в потоці апскейлу: GPU-час кадру (мітки часу D3D11), момент показу кадру на
//! екрані (статистика DXGI) і процесорний час потоку.

use super::d3d::Gpu;
use super::os;
use crate::error::UpscaleError;
use crate::measure::qpc_to_100ns;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Query, D3D11_ASYNC_GETDATA_DONOTFLUSH, D3D11_QUERY_DATA_TIMESTAMP_DISJOINT,
    D3D11_QUERY_DESC, D3D11_QUERY_TIMESTAMP, D3D11_QUERY_TIMESTAMP_DISJOINT,
};
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain1, DXGI_FRAME_STATISTICS};
use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};

/// Скільки кадрів можуть бути «в дорозі» на GPU, поки ми читаємо результати старіших.
const SLOTS: usize = 6;

struct Slot {
    disjoint: ID3D11Query,
    begin: ID3D11Query,
    end: ID3D11Query,
    pending: bool,
}

/// GPU-час кадру: від початку копіювання кадру гри до кінця масштабування.
/// Результати читаються без очікування (кілька кадрів потому), тож рендер не гальмує.
pub(super) struct GpuTimer {
    slots: Vec<Slot>,
    current: usize,
}

fn query(gpu: &Gpu, kind: D3D11_QUERY_DESC) -> Result<ID3D11Query, UpscaleError> {
    let mut q = None;
    // SAFETY: опис локальний.
    unsafe { gpu.device.CreateQuery(&kind, Some(&mut q)) }.map_err(os("CreateQuery"))?;
    q.ok_or(UpscaleError::Os {
        api: "CreateQuery",
        code: 0,
    })
}

impl GpuTimer {
    pub fn new(gpu: &Gpu) -> Result<Self, UpscaleError> {
        let desc = |q| D3D11_QUERY_DESC {
            Query: q,
            MiscFlags: 0,
        };
        let slots = (0..SLOTS)
            .map(|_| {
                Ok(Slot {
                    disjoint: query(gpu, desc(D3D11_QUERY_TIMESTAMP_DISJOINT))?,
                    begin: query(gpu, desc(D3D11_QUERY_TIMESTAMP))?,
                    end: query(gpu, desc(D3D11_QUERY_TIMESTAMP))?,
                    pending: false,
                })
            })
            .collect::<Result<Vec<_>, UpscaleError>>()?;
        Ok(Self { slots, current: 0 })
    }

    /// Початок заміру кадру. Якщо слот ще зайнятий (GPU сильно відстає) — кадр не міряємо.
    pub fn begin(&mut self, gpu: &Gpu) -> bool {
        let Some(slot) = self.slots.get(self.current) else {
            return false;
        };
        if slot.pending {
            return false;
        }
        // SAFETY: запити створені на цьому пристрої.
        unsafe {
            gpu.context.Begin(&slot.disjoint);
            gpu.context.End(&slot.begin);
        }
        true
    }

    /// Кінець заміру кадру (лише після успішного `begin`).
    pub fn end(&mut self, gpu: &Gpu) {
        let Some(slot) = self.slots.get_mut(self.current) else {
            return;
        };
        // SAFETY: як вище.
        unsafe {
            gpu.context.End(&slot.end);
            gpu.context.End(&slot.disjoint);
        }
        slot.pending = true;
        self.current = (self.current + 1) % SLOTS;
    }

    /// Готові результати (мс). Незавершені запити лишаються на наступний раз.
    pub fn collect(&mut self, gpu: &Gpu, mut on_result: impl FnMut(f64)) {
        for slot in self.slots.iter_mut().filter(|s| s.pending) {
            // «Не готово» (S_FALSE) обгортка не відрізняє від успіху, тому дані попередньо
            // заповнюємо значеннями-маркерами: не перезаписані — результату ще немає.
            let mut dj = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT {
                Frequency: 0,
                Disjoint: true.into(),
            };
            let mut t0 = u64::MAX;
            let mut t1 = u64::MAX;
            // SAFETY: буфери локальні й точно потрібного розміру для кожного типу запиту.
            unsafe {
                let flags = D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32;
                let _ = gpu.context.GetData(
                    &slot.disjoint,
                    Some((&mut dj as *mut D3D11_QUERY_DATA_TIMESTAMP_DISJOINT).cast()),
                    core::mem::size_of::<D3D11_QUERY_DATA_TIMESTAMP_DISJOINT>() as u32,
                    flags,
                );
                if dj.Frequency == 0 {
                    continue;
                }
                let _ =
                    gpu.context
                        .GetData(&slot.begin, Some((&mut t0 as *mut u64).cast()), 8, flags);
                let _ =
                    gpu.context
                        .GetData(&slot.end, Some((&mut t1 as *mut u64).cast()), 8, flags);
            }
            if t0 == u64::MAX || t1 == u64::MAX {
                continue;
            }
            slot.pending = false;
            // Disjoint — GPU змінював частоту / був перерваний: замір недостовірний.
            if !dj.Disjoint.as_bool() && t1 >= t0 {
                on_result((t1 - t0) as f64 * 1000.0 / dj.Frequency as f64);
            }
        }
    }
}

/// Відповідність «номер показу → запланований момент показу кадру гри без апскейлу», щоб за
/// статистикою DXGI дізнатися, на скільки пізніше наш кадр реально з'явився на екрані.
pub(super) struct DisplayTracker {
    recent: [(u32, i64); 32],
    next: usize,
    frequency: i64,
    last_reported: u32,
    /// Статистика недоступна (наприклад, вікно не на екрані) — перестаємо питати.
    unavailable_streak: u32,
}

impl DisplayTracker {
    pub fn new(frequency: i64) -> Self {
        Self {
            recent: [(0, 0); 32],
            next: 0,
            frequency,
            last_reported: 0,
            unavailable_streak: 0,
        }
    }

    /// Запам'ятовує, що останній показ несе кадр гри із запланованим моментом показу `scheduled_hns`.
    pub fn presented(&mut self, swap: &IDXGISwapChain1, scheduled_hns: i64) {
        // SAFETY: swap chain дійсний.
        if let Ok(count) = unsafe { swap.GetLastPresentCount() } {
            self.recent[self.next] = (count, scheduled_hns);
            self.next = (self.next + 1) % self.recent.len();
        }
    }

    /// Додана затримка (мс): наш кадр на екрані мінус запланований показ кадру гри без апскейлу.
    pub fn poll(&mut self, swap: &IDXGISwapChain1) -> Option<f64> {
        if self.unavailable_streak > 200 {
            return None;
        }
        let mut stats = DXGI_FRAME_STATISTICS::default();
        // SAFETY: локальна структура.
        if unsafe { swap.GetFrameStatistics(&mut stats) }.is_err() || stats.SyncQPCTime == 0 {
            self.unavailable_streak += 1;
            return None;
        }
        self.unavailable_streak = 0;
        if stats.PresentCount == self.last_reported {
            return None;
        }
        let &(_, scheduled) = self
            .recent
            .iter()
            .find(|(count, t)| *count == stats.PresentCount && *t != 0)?;
        self.last_reported = stats.PresentCount;
        let shown = qpc_to_100ns(stats.SyncQPCTime, self.frequency);
        Some((shown - scheduled) as f64 / 10_000.0)
    }
}

/// Процесорний час поточного потоку (ядро + користувач), одиниці по 100 нс.
pub(super) fn thread_cpu_time() -> u64 {
    let ft = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: псевдодескриптор поточного потоку; структури локальні.
    match unsafe { GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u) } {
        Ok(()) => ft(k) + ft(u),
        Err(_) => 0,
    }
}
