//! Виклики WinAPI до драйвера PawnIO.

use super::{exec_input, exec_output};
use crate::error::CoreError;
use windows::core::{w, Error as WinError};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::System::IO::DeviceIoControl;

/// Тип пристрою PawnIO і коди керування (`CTL_CODE(DEVICE_TYPE, fn, METHOD_BUFFERED, FILE_ANY_ACCESS)`).
const DEVICE_TYPE: u32 = 41_394 << 16;
const IOCTL_LOAD_BINARY: u32 = DEVICE_TYPE | (0x821 << 2);
const IOCTL_EXECUTE_FN: u32 = DEVICE_TYPE | (0x841 << 2);
/// `GENERIC_READ | GENERIC_WRITE`.
const READ_WRITE: u32 = 0xC000_0000;

fn os(api: &'static str, e: &WinError) -> CoreError {
    // HRESULT_FROM_WIN32: молодші 16 біт — код Win32.
    CoreError::Os {
        api,
        code: (e.code().0 as u32) & 0xFFFF,
    }
}

/// Закриває дескриптор рівно один раз.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: дескриптор отримано від CreateFileW/CreateMutexW і більше ніде не закривається.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Завантажений у драйвер модуль: власний дескриптор пристрою (один модуль на дескриптор).
pub struct Module {
    device: Owned,
}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PawnIO module")
    }
}

// SAFETY: дескриптор пристрою ядра дійсний у всьому процесі, а DeviceIoControl можна викликати
// з будь-якого потоку; спільного змінного стану в `Module` немає (`Sync` не реалізовано).
unsafe impl Send for Module {}

impl Module {
    /// Відкриває драйвер і завантажує підписаний модуль. Помилка — драйвера немає, немає прав
    /// або драйвер відхилив модуль (підпис).
    pub fn load(blob: &[u8]) -> Result<Self, CoreError> {
        // SAFETY: шлях — статичний нуль-термінований рядок; решта аргументів — значення.
        let handle = unsafe {
            CreateFileW(
                w!(r"\\?\GLOBALROOT\Device\PawnIO"),
                READ_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|e| os("CreateFileW(PawnIO)", &e))?;
        let device = Owned(handle);
        let len = u32::try_from(blob.len())
            .map_err(|_| CoreError::InvalidData("модуль PawnIO завеликий".into()))?;
        // SAFETY: вхідний буфер живий на час виклику, його довжина передана точно; виходу немає.
        unsafe {
            DeviceIoControl(
                device.0,
                IOCTL_LOAD_BINARY,
                Some(blob.as_ptr().cast()),
                len,
                None,
                0,
                None,
                None,
            )
        }
        .map_err(|e| os("DeviceIoControl(LOAD_BINARY)", &e))?;
        Ok(Self { device })
    }

    /// Викликає функцію модуля з аргументами; `out_len` — скільки `i64` очікується.
    pub fn execute(&self, name: &str, args: &[i64], out_len: usize) -> Result<Vec<i64>, CoreError> {
        let input = exec_input(name, args)?;
        let mut output = vec![0u8; out_len * 8];
        let in_len = u32::try_from(input.len())
            .map_err(|_| CoreError::InvalidData("завеликий вхід PawnIO".into()))?;
        let out_cap = u32::try_from(output.len())
            .map_err(|_| CoreError::InvalidData("завеликий вихід PawnIO".into()))?;
        let mut returned = 0u32;
        let out_ptr = (!output.is_empty()).then(|| output.as_mut_ptr().cast());
        // SAFETY: обидва буфери живі на час виклику, довжини передані точно; `returned` — валідний
        // вихідний вказівник. Драйвер пише не більше `out_cap` байтів.
        unsafe {
            DeviceIoControl(
                self.device.0,
                IOCTL_EXECUTE_FN,
                Some(input.as_ptr().cast()),
                in_len,
                out_ptr,
                out_cap,
                Some(&mut returned),
                None,
            )
        }
        .map_err(|e| os("DeviceIoControl(EXECUTE_FN)", &e))?;
        let n = (returned as usize).min(output.len());
        Ok(exec_output(&output[..n]))
    }
}

/// Спільний для утиліт м'ютекс доступу до PCI (`Global\Access_PCI`): його беруть HWiNFO,
/// LibreHardwareMonitor, ZenTimings. Без нього два процеси можуть переплутати пари
/// «адреса → дані» в непрямому доступі до SMN. Звільняється в `Drop` у тому самому потоці.
pub struct PciLock {
    mutex: Owned,
    _same_thread: std::marker::PhantomData<*const ()>,
}

impl PciLock {
    /// Чекає на м'ютекс до `timeout_ms`. Помилка — не дочекались (хтось тримає довго).
    pub fn acquire(timeout_ms: u32) -> Result<Self, CoreError> {
        // SAFETY: ім'я — статичний нуль-термінований рядок; атрибутів безпеки немає.
        let handle = unsafe { CreateMutexW(None, false, w!(r"Global\Access_PCI")) }
            .map_err(|e| os("CreateMutexW(Access_PCI)", &e))?;
        let mutex = Owned(handle);
        // SAFETY: дескриптор м'ютекса валідний, поки живе `mutex`.
        let wait = unsafe { WaitForSingleObject(mutex.0, timeout_ms) };
        // Покинутий м'ютекс (власник упав) теж стає нашим — доступ безпечний.
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            Ok(Self {
                mutex,
                _same_thread: std::marker::PhantomData,
            })
        } else {
            Err(CoreError::Os {
                api: "WaitForSingleObject(Access_PCI)",
                code: wait.0,
            })
        }
    }
}

impl Drop for PciLock {
    fn drop(&mut self) {
        // SAFETY: м'ютекс захоплено цим потоком в `acquire`; звільняється рівно один раз.
        let _ = unsafe { ReleaseMutex(self.mutex.0) };
    }
}
