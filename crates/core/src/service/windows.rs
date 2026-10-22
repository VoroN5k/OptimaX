//! Реалізація [`ServiceApi`] через Service Control Manager.
#![allow(unsafe_code)]

use super::{ServiceApi, ServiceState};
use crate::error::CoreError;
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_NOT_ACTIVE};
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
    StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_CONTROL_STOP,
    SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS, SERVICE_STATUS_PROCESS,
    SERVICE_STOP, SERVICE_STOPPED,
};

/// Скільки чекати переходу служби в потрібний стан.
const WAIT: Duration = Duration::from_secs(20);

/// Служби Windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinServices;

struct Sc(SC_HANDLE);

impl Drop for Sc {
    fn drop(&mut self) {
        // SAFETY: дескриптор SCM/служби закривається один раз.
        let _ = unsafe { CloseServiceHandle(self.0) };
    }
}

fn os(api: &'static str, e: &windows::core::Error) -> CoreError {
    CoreError::Os {
        api,
        code: (e.code().0 as u32) & 0xFFFF,
    }
}

fn open(name: &str, access: u32) -> Result<Sc, CoreError> {
    // SAFETY: null-рядки означають локальну машину й активну базу.
    let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
        .map_err(|e| os("OpenSCManagerW", &e))?;
    let scm = Sc(scm);
    // SAFETY: ім'я нуль-терміноване, дескриптор SCM валідний.
    let svc = unsafe { OpenServiceW(scm.0, &HSTRING::from(name), access) }
        .map_err(|e| os("OpenServiceW", &e))?;
    Ok(Sc(svc))
}

fn query(svc: &Sc) -> Result<ServiceState, CoreError> {
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0u32;
    // SAFETY: буфер — локальна структура рівно потрібного розміру.
    let buf = unsafe {
        std::slice::from_raw_parts_mut(
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
            std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
        )
    };
    // SAFETY: дескриптор має SERVICE_QUERY_STATUS, буфер валідний.
    unsafe { QueryServiceStatusEx(svc.0, SC_STATUS_PROCESS_INFO, Some(buf), &mut needed) }
        .map_err(|e| os("QueryServiceStatusEx", &e))?;
    Ok(match status.dwCurrentState {
        SERVICE_RUNNING => ServiceState::Running,
        SERVICE_STOPPED => ServiceState::Stopped,
        other => ServiceState::Other(other.0),
    })
}

fn wait_for(svc: &Sc, target: ServiceState) -> Result<(), CoreError> {
    let deadline = Instant::now() + WAIT;
    loop {
        if query(svc)? == target {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(CoreError::InvalidData(format!(
                "служба не перейшла в стан {target:?} за {} с",
                WAIT.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

impl ServiceApi for WinServices {
    fn state(&self, name: &str) -> Result<ServiceState, CoreError> {
        query(&open(name, SERVICE_QUERY_STATUS)?)
    }

    fn stop(&self, name: &str) -> Result<(), CoreError> {
        let svc = open(name, SERVICE_STOP | SERVICE_QUERY_STATUS)?;
        let mut status = SERVICE_STATUS::default();
        // SAFETY: дескриптор має SERVICE_STOP. Залежні служби НЕ зупиняємо: якщо вони є,
        // система поверне ERROR_DEPENDENT_SERVICES_RUNNING, і ми відмовимось.
        if let Err(e) = unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut status) } {
            let err = os("ControlService", &e);
            if !matches!(err, CoreError::Os { code, .. } if code == ERROR_SERVICE_NOT_ACTIVE.0) {
                return Err(err);
            }
        }
        wait_for(&svc, ServiceState::Stopped)
    }

    fn start(&self, name: &str) -> Result<(), CoreError> {
        let svc = open(name, SERVICE_START | SERVICE_QUERY_STATUS)?;
        // SAFETY: дескриптор має SERVICE_START, аргументів немає.
        if let Err(e) = unsafe { StartServiceW(svc.0, None) } {
            let err = os("StartServiceW", &e);
            if !matches!(err, CoreError::Os { code, .. } if code == ERROR_SERVICE_ALREADY_RUNNING.0)
            {
                return Err(err);
            }
        }
        wait_for(&svc, ServiceState::Running)
    }
}
