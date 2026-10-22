//! Стан RAM і очищення standby-списку за трейтом [`MemoryApi`].
//!
//! Очищення standby — **недокументований** виклик `NtSetSystemInformation` (так роблять RAMMap
//! та ISLC). На ПК з достатнім обсягом RAM ефекту зазвичай немає: Windows і так віддає
//! кешовану пам'ять за потреби. Тому виконується лише коли вільної пам'яті справді мало.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::WinMemory;

/// Абсолютний поріг «пам'яті мало».
pub const LOW_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
/// Відносний поріг «пам'яті мало», частка від усієї RAM.
pub const LOW_MEMORY_FRACTION: f64 = 0.05;

/// Стан фізичної пам'яті.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryStatus {
    /// Усього, байт.
    pub total_bytes: u64,
    /// Доступно (вільна + standby), байт.
    pub available_bytes: u64,
}

impl MemoryStatus {
    /// Чи доступної пам'яті менше за поріг (1 ГіБ або 5 % — що більше).
    pub fn is_low(&self) -> bool {
        let fraction = (self.total_bytes as f64 * LOW_MEMORY_FRACTION) as u64;
        self.available_bytes < LOW_MEMORY_BYTES.max(fraction)
    }
}

/// Доступ до пам'яті.
pub trait MemoryApi {
    /// Поточний стан.
    fn status(&self) -> Result<MemoryStatus, CoreError>;
    /// Очищає standby-список. Потребує прав адміністратора.
    fn purge_standby(&self) -> Result<(), CoreError>;
}

/// Пам'ять для тестів: фіксований стан, лічильник очищень.
#[derive(Debug)]
pub struct FakeMemory {
    /// Стан, що повертається.
    pub status: MemoryStatus,
    purges: AtomicU32,
}

impl FakeMemory {
    /// Новий мок.
    pub fn new(total_bytes: u64, available_bytes: u64) -> Self {
        Self {
            status: MemoryStatus {
                total_bytes,
                available_bytes,
            },
            purges: AtomicU32::new(0),
        }
    }

    /// Скільки разів викликали очищення.
    pub fn purges(&self) -> u32 {
        self.purges.load(Ordering::SeqCst)
    }
}

impl MemoryApi for FakeMemory {
    fn status(&self) -> Result<MemoryStatus, CoreError> {
        Ok(self.status)
    }

    fn purge_standby(&self) -> Result<(), CoreError> {
        self.purges.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn low_memory_threshold() {
        let s = |total, avail| MemoryStatus {
            total_bytes: total,
            available_bytes: avail,
        };
        // 32 ГБ: поріг 5 % = 1.6 ГБ.
        assert!(s(32 * GIB, GIB).is_low());
        assert!(!s(32 * GIB, 2 * GIB).is_low());
        // 8 ГБ: поріг 1 ГБ (5 % = 0.4 ГБ менше).
        assert!(s(8 * GIB, GIB / 2).is_low());
        assert!(!s(8 * GIB, 3 * GIB / 2).is_low());
    }
}
