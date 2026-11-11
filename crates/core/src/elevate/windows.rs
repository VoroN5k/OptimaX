//! `ShellExecuteExW` з дієсловом `runas`.
#![allow(unsafe_code)]

use crate::CoreError;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: дескриптор процесу отримано від ShellExecuteExW і закривається один раз.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

fn os(api: &'static str, e: &windows::core::Error) -> CoreError {
    // HRESULT_FROM_WIN32: молодші 16 біт — код Win32.
    CoreError::Os {
        api,
        code: (e.code().0 as u32) & 0xFFFF,
    }
}

pub(super) fn run_elevated(params: &str, wait: bool) -> Result<Option<u32>, CoreError> {
    let exe = std::env::current_exe().map_err(|e| CoreError::NotFound(e.to_string()))?;
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(params);
    let verb = HSTRING::from("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: структура заповнена, рядки живуть до кінця виклику.
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| os("ShellExecuteExW", &e))?;
    let process = Process(info.hProcess);
    if !wait || process.0.is_invalid() {
        return Ok(None);
    }
    // SAFETY: дескриптор процесу валідний (SEE_MASK_NOCLOSEPROCESS).
    if unsafe { WaitForSingleObject(process.0, INFINITE) } != WAIT_OBJECT_0 {
        return Err(CoreError::Os {
            api: "WaitForSingleObject",
            code: 0,
        });
    }
    let mut code = 0u32;
    // SAFETY: дескриптор валідний, процес завершився.
    unsafe { GetExitCodeProcess(process.0, &mut code) }
        .map_err(|e| os("GetExitCodeProcess", &e))?;
    Ok(Some(code))
}
