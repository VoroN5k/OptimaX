//! Анатомія кадру: власне трасування ETW на шкалі QPC (та сама, що в кадрів PresentMon).
//!
//! Етап 0 (розвідка): загальна сесія трасування ([`Session`]) — системна (події ядра за
//! прапорцями: перемикання й готовність потоків, потоки й процеси, диск, жорсткі page faults,
//! DPC/ISR) або звичайна (провайдери за GUID, напр. Microsoft-Windows-DxgKrnl і
//! Microsoft-Windows-DXGI), і [`EventStats`] — підрахунок подій за видами, щоб виміряти обсяг і
//! ціну трасування на реальному ПК (приклад `trace_probe`). Розбір подій — наступні етапи.
//!
//! Нічого в системі не змінює. Потрібні права адміністратора.

#![deny(unsafe_code)]

pub mod threads;
#[cfg(windows)]
mod tracer;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use tracer::ThreadTracer;
#[cfg(windows)]
pub use windows::{event_names, provider_guid, Record, Session};

use std::collections::HashMap;

/// Помилка трасування.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// Виклик Windows API повернув помилку.
    #[error("{call}: код {code}")]
    Api {
        /// Функція.
        call: &'static str,
        /// Код помилки Win32.
        code: u32,
    },
    /// Трасування доступне лише на Windows.
    #[error("трасування ETW доступне лише на Windows")]
    Unsupported,
}

/// Провайдер звичайної сесії.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSpec {
    /// GUID провайдера.
    pub guid: u128,
    /// Рівень (4 — інформаційний, 5 — докладний).
    pub level: u8,
    /// Ключові слова (`MatchAnyKeyword`).
    pub keywords: u64,
}

/// Що трасувати.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionKind {
    /// Системна сесія з подіями ядра (`EVENT_TRACE_SYSTEM_LOGGER_MODE`, Windows 8+).
    Kernel {
        /// Прапорці `EVENT_TRACE_FLAG_*`.
        flags: u32,
    },
    /// Звичайна сесія з провайдерами.
    Providers(Vec<ProviderSpec>),
}

/// Вид події: провайдер, ідентифікатор, код операції, версія.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventKey {
    /// GUID провайдера (для класичних подій ядра — GUID класу, напр. `Thread`).
    pub provider: u128,
    /// Ідентифікатор події (у класичних подій ядра зазвичай 0).
    pub id: u16,
    /// Код операції (у класичних подій ядра — вид події, напр. 36 у `Thread` — CSwitch).
    pub opcode: u8,
    /// Версія.
    pub version: u8,
}

/// Лічильник одного виду подій.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindStats {
    /// Скільки подій.
    pub count: u64,
    /// Скільки з них від процесу, за яким стежимо (за заголовком події).
    pub of_target: u64,
    /// Сумарний обсяг даних подій, байт.
    pub bytes: u64,
}

/// Підрахунок подій за видами (чиста структура — для заміру обсягу трасування).
#[derive(Debug, Clone, Default)]
pub struct EventStats {
    kinds: HashMap<EventKey, KindStats>,
    target_pid: Option<u32>,
}

impl EventStats {
    /// Лічильник; `target_pid` — процес, події якого рахувати окремо.
    pub fn new(target_pid: Option<u32>) -> Self {
        Self {
            kinds: HashMap::new(),
            target_pid,
        }
    }

    /// Враховує подію. Повертає `true`, якщо такий вид трапився вперше.
    pub fn add(&mut self, key: EventKey, pid: u32, bytes: usize) -> bool {
        let first = !self.kinds.contains_key(&key);
        let k = self.kinds.entry(key).or_default();
        k.count += 1;
        k.bytes += bytes as u64;
        if self.target_pid == Some(pid) {
            k.of_target += 1;
        }
        first
    }

    /// Усього подій.
    pub fn total(&self) -> u64 {
        self.kinds.values().map(|k| k.count).sum()
    }

    /// Усього байт даних.
    pub fn total_bytes(&self) -> u64 {
        self.kinds.values().map(|k| k.bytes).sum()
    }

    /// Види подій, найчастіші першими.
    pub fn sorted(&self) -> Vec<(EventKey, KindStats)> {
        let mut v: Vec<(EventKey, KindStats)> =
            self.kinds.iter().map(|(k, s)| (*k, s.clone())).collect();
        v.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(&b.0)));
        v
    }
}

/// GUID у звичному записі `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
pub fn guid_string(g: u128) -> String {
    let h = format!("{g:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(opcode: u8) -> EventKey {
        EventKey {
            provider: 1,
            id: 0,
            opcode,
            version: 2,
        }
    }

    #[test]
    fn stats_count_kinds_target_and_bytes() {
        let mut s = EventStats::new(Some(42));
        assert!(s.add(key(36), 42, 24));
        assert!(!s.add(key(36), 7, 24));
        assert!(s.add(key(50), 42, 16));
        assert_eq!(s.total(), 3);
        assert_eq!(s.total_bytes(), 64);
        let sorted = s.sorted();
        assert_eq!(sorted[0].0.opcode, 36);
        assert_eq!(
            sorted[0].1,
            KindStats {
                count: 2,
                of_target: 1,
                bytes: 48
            }
        );
    }

    #[test]
    fn guid_text() {
        assert_eq!(
            guid_string(0x3d6fa8d1_fe05_11d0_9dda_00c04fd7ba7c),
            "3d6fa8d1-fe05-11d0-9dda-00c04fd7ba7c"
        );
    }
}
