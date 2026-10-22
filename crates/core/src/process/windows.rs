//! Реалізація [`ProcessApi`] через Toolhelp32 і Process API.
#![allow(unsafe_code)]

use super::{PriorityClass, ProcessApi, ProcessInfo};
use crate::error::CoreError;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetPriorityClass, GetProcessTimes, OpenProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
    BELOW_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    PROCESS_ACCESS_RIGHTS, PROCESS_CREATION_FLAGS, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SET_INFORMATION, REALTIME_PRIORITY_CLASS,
};

/// Процеси Windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinProcesses;

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: дескриптор отримано від системи й закривається один раз.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn creation_time(h: HANDLE) -> Option<u64> {
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: дескриптор відкрито з PROCESS_QUERY_LIMITED_INFORMATION, вказівники — на локальні змінні.
    unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) }.ok()?;
    Some((u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime))
}

/// Відкриває процес і перевіряє, що це той самий процес (час створення збігається).
fn open_same(p: &ProcessInfo, access: PROCESS_ACCESS_RIGHTS) -> Result<Handle, CoreError> {
    let gone = || CoreError::NotFound(format!("процес {} ({})", p.name, p.pid));
    // SAFETY: звичайний виклик; дескриптор обгортається в Handle.
    let h = unsafe { OpenProcess(access | PROCESS_QUERY_LIMITED_INFORMATION, false, p.pid) }
        .map_err(|_| gone())?;
    let h = Handle(h);
    if creation_time(h.0) != Some(p.start_time) {
        return Err(gone());
    }
    Ok(h)
}

fn to_flags(p: PriorityClass) -> PROCESS_CREATION_FLAGS {
    match p {
        PriorityClass::Idle => IDLE_PRIORITY_CLASS,
        PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
        PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
        PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        PriorityClass::High => HIGH_PRIORITY_CLASS,
        PriorityClass::Realtime => REALTIME_PRIORITY_CLASS,
    }
}

fn from_flags(v: u32) -> Option<PriorityClass> {
    [
        PriorityClass::Idle,
        PriorityClass::BelowNormal,
        PriorityClass::Normal,
        PriorityClass::AboveNormal,
        PriorityClass::High,
        PriorityClass::Realtime,
    ]
    .into_iter()
    .find(|p| to_flags(*p).0 == v)
}

impl ProcessApi for WinProcesses {
    fn find_by_name(&self, name: &str) -> Result<Vec<ProcessInfo>, CoreError> {
        // SAFETY: знімок усіх процесів; дескриптор обгортається в Handle.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(|e| {
            CoreError::Os {
                api: "CreateToolhelp32Snapshot",
                code: e.code().0 as u32,
            }
        })?;
        let snap = Handle(snap);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut out = Vec::new();
        // SAFETY: `entry` має правильний dwSize; перелік закінчується помилкою.
        let mut ok = unsafe { Process32FirstW(snap.0, &mut entry) }.is_ok();
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if exe.eq_ignore_ascii_case(name) {
                // Захищені процеси не відкриваються — їх просто пропускаємо.
                // SAFETY: звичайний виклик; дескриптор обгортається в Handle.
                if let Ok(h) = unsafe {
                    OpenProcess(
                        PROCESS_QUERY_LIMITED_INFORMATION,
                        false,
                        entry.th32ProcessID,
                    )
                } {
                    let h = Handle(h);
                    if let Some(start_time) = creation_time(h.0) {
                        out.push(ProcessInfo {
                            pid: entry.th32ProcessID,
                            name: exe,
                            start_time,
                        });
                    }
                }
            }
            // SAFETY: як вище.
            ok = unsafe { Process32NextW(snap.0, &mut entry) }.is_ok();
        }
        Ok(out)
    }

    fn priority(&self, process: &ProcessInfo) -> Result<PriorityClass, CoreError> {
        let h = open_same(process, PROCESS_QUERY_LIMITED_INFORMATION)?;
        // SAFETY: дескриптор валідний і має право запиту.
        let v = unsafe { GetPriorityClass(h.0) };
        from_flags(v)
            .ok_or_else(|| CoreError::InvalidData(format!("невідомий клас пріоритету {v}")))
    }

    fn set_priority(
        &self,
        process: &ProcessInfo,
        priority: PriorityClass,
    ) -> Result<(), CoreError> {
        let h = open_same(process, PROCESS_SET_INFORMATION)?;
        // SAFETY: дескриптор валідний і має PROCESS_SET_INFORMATION.
        unsafe { SetPriorityClass(h.0, to_flags(priority)) }.map_err(|e| CoreError::Os {
            api: "SetPriorityClass",
            code: e.code().0 as u32,
        })
    }
}
