//! Один екземпляр процесу на сеанс користувача (агент у треї, вікно програми): іменований м'ютекс
//! у просторі імен сеансу (`Local\…`). Поки живе [`InstanceGuard`], другий [`acquire`] з тим самим
//! іменем повертає `None`. М'ютекс, створений процесом з правами адміністратора, процес без прав
//! відкрити не може — «доступ заборонено» теж означає «зайнято».

use crate::error::CoreError;

/// Виводить на передній план вікно верхнього рівня з заголовком `title` (згорнуте — розгортає).
/// `false` — такого вікна немає.
pub fn activate_window(title: &str) -> bool {
    imp::activate_window(title)
}

/// Дозволяє будь-якому процесу (напр. щойно запущеному вікну програми) вивести своє вікно на
/// передній план. Діє, лише якщо цей процес сам має таке право (щойно був клік у треї).
pub fn allow_foreground_any() {
    imp::allow_foreground_any();
}

/// Тримає ім'я зайнятим, поки живе.
#[derive(Debug)]
pub struct InstanceGuard {
    /// Звільняє ім'я при знищенні.
    _inner: imp::Guard,
}

/// Займає ім'я `name` (напр. `Local\OptimaX-Agent`). `Ok(None)` — уже зайняте іншим процесом.
pub fn acquire(name: &str) -> Result<Option<InstanceGuard>, CoreError> {
    Ok(imp::acquire(name)?.map(|inner| InstanceGuard { _inner: inner }))
}

/// Чи зайняте ім'я зараз (іншим процесом або цим).
pub fn is_taken(name: &str) -> bool {
    imp::is_taken(name)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use crate::error::CoreError;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, HANDLE,
    };
    use windows::Win32::System::Threading::{
        CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, ASFW_ANY,
        SW_RESTORE, SW_SHOW,
    };

    pub(super) fn activate_window(title: &str) -> bool {
        // SAFETY: лише пошук вікна за заголовком; ShowWindow/SetForegroundWindow — з дійсним
        // дескриптором, помилки (напр. чуже вікно з вищими правами) ігноруються.
        unsafe {
            let Ok(hwnd) = FindWindowW(None, &HSTRING::from(title)) else {
                return false;
            };
            if hwnd.is_invalid() {
                return false;
            }
            let _ = ShowWindow(
                hwnd,
                if IsIconic(hwnd).as_bool() {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            let _ = SetForegroundWindow(hwnd);
            true
        }
    }

    pub(super) fn allow_foreground_any() {
        // SAFETY: без вказівників; без права — просто помилка, яку ігноруємо.
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }

    #[derive(Debug)]
    pub(super) struct Guard(HANDLE);

    // SAFETY: дескриптор м'ютексу дійсний у всьому процесі; ми його лише закриваємо.
    unsafe impl Send for Guard {}
    // SAFETY: як вище — спільного змінного стану немає.
    unsafe impl Sync for Guard {}

    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: дескриптор від CreateMutexW, закривається один раз.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn acquire(name: &str) -> Result<Option<Guard>, CoreError> {
        // SAFETY: ім'я — HSTRING; без дескриптора безпеки (типовий для сеансу).
        let handle = match unsafe { CreateMutexW(None, false, &HSTRING::from(name)) } {
            Ok(h) => h,
            // Існує, але створений процесом з правами адміністратора — зайнято.
            Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => return Ok(None),
            Err(e) => {
                return Err(CoreError::Os {
                    api: "CreateMutexW",
                    code: e.code().0 as u32,
                })
            }
        };
        // SAFETY: без аргументів; одразу після CreateMutexW.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: дескриптор щойно отримано.
            let _ = unsafe { CloseHandle(handle) };
            return Ok(None);
        }
        Ok(Some(Guard(handle)))
    }

    pub(super) fn is_taken(name: &str) -> bool {
        // SAFETY: ім'я — HSTRING; лише перевірка існування.
        match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from(name)) } {
            Ok(h) => {
                // SAFETY: дескриптор щойно відкрито.
                let _ = unsafe { CloseHandle(h) };
                true
            }
            Err(e) => e.code() == ERROR_ACCESS_DENIED.to_hresult(),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::error::CoreError;

    #[derive(Debug)]
    pub(super) struct Guard;

    pub(super) fn acquire(_name: &str) -> Result<Option<Guard>, CoreError> {
        Err(CoreError::Unsupported("один екземпляр"))
    }

    pub(super) fn is_taken(_name: &str) -> bool {
        false
    }

    pub(super) fn activate_window(_title: &str) -> bool {
        false
    }

    pub(super) fn allow_foreground_any() {}
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_until_the_first_is_dropped() {
        let name = format!(r"Local\OptimaX-test-{}", std::process::id());
        assert!(!is_taken(&name));
        let first = acquire(&name).expect("перший").expect("вільне");
        assert!(is_taken(&name));
        assert!(acquire(&name).expect("другий").is_none());
        drop(first);
        assert!(!is_taken(&name));
        assert!(acquire(&name).expect("знову").is_some());
    }
}
