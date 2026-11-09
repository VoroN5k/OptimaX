//! Іменований канал Windows: сервер із дескриптором безпеки, клієнт, процес на іншому кінці.
//! Синхронний байтовий режим; кожне з'єднання обслуговує свій потік.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL,
    INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Доступ клієнта: `FILE_GENERIC_READ | FILE_WRITE_DATA` — рівно те, що дає дескриптор каналу
/// користувачу (`crate::pipe_sddl`). `GENERIC_WRITE` просив би ще й `FILE_APPEND_DATA`
/// (= створення екземплярів каналу), якого користувач свідомо не має.
const CLIENT_ACCESS: u32 = 0x0012_008B;
const BUF: u32 = 64 * 1024;

fn io_err(e: &windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
}

/// Кінець каналу (сервера чи клієнта).
pub struct PipeStream {
    handle: HANDLE,
    server: bool,
}

// SAFETY: дескриптор об'єкта ядра дійсний у всьому процесі; ReadFile/WriteFile з різних потоків
// на одному дескрипторі безпечні. Спільного змінного стану в Rust-обгортці немає.
unsafe impl Send for PipeStream {}

impl std::fmt::Debug for PipeStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.server {
            "pipe server"
        } else {
            "pipe client"
        })
    }
}

/// Пам'ять від LocalAlloc (дескриптор безпеки з SDDL).
struct Local(PSECURITY_DESCRIPTOR);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            // SAFETY: виділено ConvertStringSecurityDescriptorToSecurityDescriptorW; звільняється раз.
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0 .0))) };
        }
    }
}

impl PipeStream {
    /// Створює екземпляр каналу з дескриптором безпеки `sddl`, ще не з'єднаний. `first` — першим
    /// екземпляром (якщо канал із таким іменем уже хтось створив — помилка, а не спільний канал).
    pub fn listen(name: &str, sddl: &str, first: bool) -> io::Result<Self> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: SDDL — валідний рядок, `sd` — вихідний параметр (звільняється через `Local`).
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .map_err(|e| io_err(&e))?;
        let _free = Local(sd);
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let mut mode = PIPE_ACCESS_DUPLEX;
        if first {
            mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: ім'я — HSTRING, `sa` і дескриптор живі до кінця виклику.
        let handle = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(name),
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUF,
                BUF,
                0,
                Some(&sa),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle,
            server: true,
        })
    }

    /// Чекає клієнта (блокує потік).
    pub fn accept(&self) -> io::Result<()> {
        // SAFETY: дескриптор серверного екземпляра; синхронний режим (без OVERLAPPED).
        match unsafe { ConnectNamedPipe(self.handle, None) } {
            Ok(()) => Ok(()),
            // Клієнт під'єднався між CreateNamedPipe і ConnectNamedPipe — теж з'єднання.
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
            Err(e) => Err(io_err(&e)),
        }
    }

    /// Під'єднується до каналу як клієнт. Сервер може лише впізнати клієнта, але не діяти від
    /// його імені (`SECURITY_IDENTIFICATION`) — на випадок, якщо канал підмінено.
    pub fn connect(name: &str) -> io::Result<Self> {
        // SAFETY: ім'я — HSTRING; решта — значення.
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(name),
                CLIENT_ACCESS,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0),
                None,
            )
        }
        .map_err(|e| io_err(&e))?;
        Ok(Self {
            handle,
            server: false,
        })
    }

    /// PID процесу-клієнта (на сервері).
    pub fn client_pid(&self) -> io::Result<u32> {
        let mut pid = 0u32;
        // SAFETY: дескриптор з'єднаного каналу; `pid` — вихідний параметр.
        unsafe { GetNamedPipeClientProcessId(self.handle, &mut pid) }.map_err(|e| io_err(&e))?;
        Ok(pid)
    }

    /// PID процесу-сервера (на клієнті).
    pub fn server_pid(&self) -> io::Result<u32> {
        let mut pid = 0u32;
        // SAFETY: дескриптор з'єднаного каналу; `pid` — вихідний параметр.
        unsafe { GetNamedPipeServerProcessId(self.handle, &mut pid) }.map_err(|e| io_err(&e))?;
        Ok(pid)
    }
}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut read = 0u32;
        // SAFETY: буфер живий на час виклику; синхронне читання.
        match unsafe { ReadFile(self.handle, Some(buf), Some(&mut read), None) } {
            Ok(()) => Ok(read as usize),
            // Друга сторона закрила канал — кінець потоку.
            Err(e) if e.code() == ERROR_BROKEN_PIPE.to_hresult() => Ok(0),
            Err(e) => Err(io_err(&e)),
        }
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut written = 0u32;
        // SAFETY: буфер живий на час виклику; синхронний запис.
        unsafe { WriteFile(self.handle, Some(buf), Some(&mut written), None) }
            .map_err(|e| io_err(&e))?;
        Ok(written as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for PipeStream {
    fn drop(&mut self) {
        if self.server {
            // SAFETY: дескриптор серверного екземпляра; дочекатися, поки клієнт дочитає, і
            // розірвати з'єднання. Помилки неважливі: канал однаково закривається.
            unsafe {
                let _ = FlushFileBuffers(self.handle);
                let _ = DisconnectNamedPipe(self.handle);
            }
        }
        // SAFETY: дескриптор отримано від CreateNamedPipeW/CreateFileW; закривається один раз.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

/// Повний шлях exe процесу `pid`.
pub fn process_image(pid: u32) -> io::Result<PathBuf> {
    // SAFETY: обмежений доступ на читання відомостей; дескриптор закривається нижче.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| io_err(&e))?;
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as u32;
    // SAFETY: буфер на `len` символів живий на час виклику.
    let r = unsafe {
        QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    };
    // SAFETY: дескриптор від OpenProcess, закривається один раз.
    let _ = unsafe { CloseHandle(h) };
    r.map_err(|e| io_err(&e))?;
    Ok(PathBuf::from(String::from_utf16_lossy(
        &buf[..len as usize],
    )))
}
