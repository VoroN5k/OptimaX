//! Процеси та їхній пріоритет за трейтом [`ProcessApi`].
//!
//! Процес ідентифікується парою PID + час створення: PID після завершення процесу може
//! отримати інший процес, і без часу створення ми змінили б пріоритет чужому.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, PoisonError};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::WinProcesses;

/// Клас пріоритету процесу Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityClass {
    /// Idle
    Idle,
    /// Below normal
    BelowNormal,
    /// Normal
    Normal,
    /// Above normal
    AboveNormal,
    /// High
    High,
    /// Realtime — OptimaX ніколи його не встановлює.
    Realtime,
}

impl PriorityClass {
    /// Ключ локалізації назви.
    pub fn text_key(self) -> &'static str {
        match self {
            Self::Idle => "priority.idle",
            Self::BelowNormal => "priority.below_normal",
            Self::Normal => "priority.normal",
            Self::AboveNormal => "priority.above_normal",
            Self::High => "priority.high",
            Self::Realtime => "priority.realtime",
        }
    }
}

/// Запущений процес.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProcessInfo {
    /// PID.
    pub pid: u32,
    /// Ім'я exe.
    pub name: String,
    /// Час створення (FILETIME) — разом із PID однозначно визначає процес.
    pub start_time: u64,
}

/// Процеси, яким OptimaX ніколи не змінює пріоритет: системні, звук, ввід, захист,
/// античити й сам OptimaX. Порівняння без урахування регістру.
pub const PROTECTED_PROCESSES: &[&str] = &[
    "system",
    "registry",
    "secure system",
    "memory compression",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    "svchost.exe",
    "dwm.exe",
    "audiodg.exe",
    "fontdrvhost.exe",
    "ctfmon.exe",
    "explorer.exe",
    "sihost.exe",
    "msmpeng.exe",
    "nissrv.exe",
    "securityhealthservice.exe",
    "easyanticheat.exe",
    "easyanticheat_eos.exe",
    "beservice.exe",
    "vgc.exe",
    "vgtray.exe",
    "faceitservice.exe",
    "faceit.exe",
    "optimax.exe",
];

/// Чи захищений процес (за ім'ям exe).
pub fn is_protected_process(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n.starts_with("presentmon") || PROTECTED_PROCESSES.contains(&n.as_str())
}

/// Доступ до процесів.
pub trait ProcessApi {
    /// Процеси з таким ім'ям exe (без урахування регістру).
    fn find_by_name(&self, name: &str) -> Result<Vec<ProcessInfo>, CoreError>;
    /// Пріоритет процесу. [`CoreError::NotFound`], якщо процес завершився.
    fn priority(&self, process: &ProcessInfo) -> Result<PriorityClass, CoreError>;
    /// Встановлює пріоритет. [`CoreError::NotFound`], якщо процес завершився.
    fn set_priority(&self, process: &ProcessInfo, priority: PriorityClass)
        -> Result<(), CoreError>;
}

#[derive(Debug, Clone)]
struct MemProc {
    info: ProcessInfo,
    priority: PriorityClass,
}

/// Процеси в пам'яті для тестів.
#[derive(Debug, Default)]
pub struct MemProcesses {
    procs: Mutex<Vec<MemProc>>,
}

impl MemProcesses {
    /// Порожній список.
    pub fn new() -> Self {
        Self::default()
    }

    /// Додає процес із пріоритетом Normal і повертає його опис.
    pub fn spawn(&self, pid: u32, name: &str, start_time: u64) -> ProcessInfo {
        let info = ProcessInfo {
            pid,
            name: name.into(),
            start_time,
        };
        self.procs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(MemProc {
                info: info.clone(),
                priority: PriorityClass::Normal,
            });
        info
    }

    /// Завершує процес.
    pub fn kill(&self, pid: u32) {
        self.procs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|p| p.info.pid != pid);
    }
}

impl ProcessApi for MemProcesses {
    fn find_by_name(&self, name: &str) -> Result<Vec<ProcessInfo>, CoreError> {
        Ok(self
            .procs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|p| p.info.name.eq_ignore_ascii_case(name))
            .map(|p| p.info.clone())
            .collect())
    }

    fn priority(&self, process: &ProcessInfo) -> Result<PriorityClass, CoreError> {
        self.procs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|p| p.info.pid == process.pid && p.info.start_time == process.start_time)
            .map(|p| p.priority)
            .ok_or_else(|| {
                CoreError::NotFound(format!("процес {} ({})", process.name, process.pid))
            })
    }

    fn set_priority(
        &self,
        process: &ProcessInfo,
        priority: PriorityClass,
    ) -> Result<(), CoreError> {
        let mut procs = self.procs.lock().unwrap_or_else(PoisonError::into_inner);
        let p = procs
            .iter_mut()
            .find(|p| p.info.pid == process.pid && p.info.start_time == process.start_time)
            .ok_or_else(|| {
                CoreError::NotFound(format!("процес {} ({})", process.name, process.pid))
            })?;
        p.priority = priority;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_names() {
        assert!(is_protected_process("AudioDG.exe"));
        assert!(is_protected_process("PresentMon-2.6.0-x64.exe"));
        assert!(!is_protected_process("chrome.exe"));
    }

    #[test]
    fn pid_reuse_is_detected() {
        let p = MemProcesses::new();
        let old = p.spawn(10, "chrome.exe", 100);
        p.kill(10);
        p.spawn(10, "other.exe", 200);
        assert!(matches!(p.priority(&old), Err(CoreError::NotFound(_))));
        assert!(p.set_priority(&old, PriorityClass::BelowNormal).is_err());
    }
}
