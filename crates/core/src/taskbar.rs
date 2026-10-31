//! Велика іконка вікна для панелі завдань.
//!
//! Tauri ставить вікну лише малу іконку (`ICON_SMALL`). Звичайній програмі вистачає: велику
//! панель завдань бере з exe. Але OptimaX працює з правами адміністратора, і Провідник (без них)
//! не може прочитати шлях до її exe — на панелі лишалася стандартна іконка «програма». Тому
//! велика іконка (`ICON_BIG`) задається явно з ресурсу власного exe.

/// Ідентифікатор ресурсу іконки, який вшиває `tauri-build`
/// (`tauri_utils::platform::WINDOWS_APP_ICON_RESOURCE_ID`).
pub const APP_ICON_RESOURCE_ID: u16 = 32512;

/// Ставить вікну `hwnd` велику іконку з ресурсу exe. Помилки ігноруються: це лише вигляд.
pub fn set_big_icon(hwnd: isize) {
    imp::set_big_icon(hwnd);
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use super::APP_ICON_RESOURCE_ID;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, LoadImageW, SendMessageW, ICON_BIG, IMAGE_ICON, LR_DEFAULTCOLOR,
        SM_CXICON, SM_CYICON, WM_SETICON,
    };

    pub(super) fn set_big_icon(hwnd: isize) {
        // SAFETY: модуль поточного процесу; дескриптор не звільняється.
        let Ok(module) = (unsafe { GetModuleHandleW(None) }) else {
            return;
        };
        // SAFETY: MAKEINTRESOURCE — числовий ідентифікатор як вказівник; розміри — системні.
        let icon = unsafe {
            LoadImageW(
                Some(module.into()),
                PCWSTR(usize::from(APP_ICON_RESOURCE_ID) as *const u16),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXICON),
                GetSystemMetrics(SM_CYICON),
                LR_DEFAULTCOLOR,
            )
        };
        let Ok(icon) = icon else {
            return;
        };
        // SAFETY: `hwnd` — вікно цього процесу; іконка живе до кінця процесу (система не копіює
        // її, тож свідомо не звільняємо).
        unsafe {
            SendMessageW(
                HWND(hwnd as *mut _),
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(icon.0 as isize)),
            );
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn set_big_icon(_hwnd: isize) {}
}
