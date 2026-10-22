//! Реалізація [`MemoryApi`]: GlobalMemoryStatusEx і (недокументоване) очищення standby.
#![allow(unsafe_code)]

use super::{MemoryApi, MemoryStatus};
use crate::error::CoreError;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_NOT_ALL_ASSIGNED, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    SE_PROF_SINGLE_PROCESS_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// `SYSTEM_INFORMATION_CLASS::SystemMemoryListInformation` (недокументовано).
const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
/// `SYSTEM_MEMORY_LIST_COMMAND::MemoryPurgeStandbyList` (недокументовано).
const MEMORY_PURGE_STANDBY_LIST: u32 = 4;

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn NtSetSystemInformation(class: u32, info: *const core::ffi::c_void, length: u32) -> i32;
}

/// Пам'ять Windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinMemory;

/// Вмикає SeProfileSingleProcessPrivilege у токені поточного процесу (є лише в адміністратора).
fn enable_privilege() -> Result<(), CoreError> {
    let mut token = HANDLE::default();
    // SAFETY: псевдодескриптор поточного процесу; токен закривається нижче.
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
    }
    .map_err(|e| CoreError::Os {
        api: "OpenProcessToken",
        code: (e.code().0 as u32) & 0xFFFF,
    })?;
    let mut luid = LUID::default();
    // SAFETY: ім'я привілею — статичний нуль-термінований рядок.
    let looked_up =
        unsafe { LookupPrivilegeValueW(PCWSTR::null(), SE_PROF_SINGLE_PROCESS_NAME, &mut luid) };
    let result = looked_up
        .map_err(|e| CoreError::Os {
            api: "LookupPrivilegeValueW",
            code: (e.code().0 as u32) & 0xFFFF,
        })
        .and_then(|()| {
            let tp = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            // SAFETY: токен відкрито з TOKEN_ADJUST_PRIVILEGES, структура валідна.
            unsafe { AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None) }.map_err(
                |e| CoreError::Os {
                    api: "AdjustTokenPrivileges",
                    code: (e.code().0 as u32) & 0xFFFF,
                },
            )?;
            // Успіх API не гарантує, що привілей надано: перевіряємо GetLastError.
            // SAFETY: читає код помилки поточного потоку.
            if unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
                return Err(CoreError::Os {
                    api: "AdjustTokenPrivileges",
                    code: ERROR_NOT_ALL_ASSIGNED.0,
                });
            }
            Ok(())
        });
    // SAFETY: токен відкрито вище й закривається один раз.
    let _ = unsafe { CloseHandle(token) };
    result
}

impl MemoryApi for WinMemory {
    fn status(&self) -> Result<MemoryStatus, CoreError> {
        let mut m = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: структура з правильним dwLength.
        unsafe { GlobalMemoryStatusEx(&mut m) }.map_err(|e| CoreError::Os {
            api: "GlobalMemoryStatusEx",
            code: (e.code().0 as u32) & 0xFFFF,
        })?;
        Ok(MemoryStatus {
            total_bytes: m.ullTotalPhys,
            available_bytes: m.ullAvailPhys,
        })
    }

    fn purge_standby(&self) -> Result<(), CoreError> {
        enable_privilege()?;
        let command = MEMORY_PURGE_STANDBY_LIST;
        // SAFETY: передаємо вказівник на u32-команду і її розмір, як очікує цей клас інформації.
        let status = unsafe {
            NtSetSystemInformation(
                SYSTEM_MEMORY_LIST_INFORMATION,
                (&command as *const u32).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        if status >= 0 {
            Ok(())
        } else {
            Err(CoreError::Os {
                api: "NtSetSystemInformation",
                code: status as u32,
            })
        }
    }
}
