//! Windows-реалізація: WGC-захоплення, Direct3D 11, вікно-оверлей.
//!
//! Усі `unsafe` — виклики WinAPI/COM з локальними або щойно отриманими значеннями;
//! кожен блок має коментар `// SAFETY:`.

mod capture;
mod cursor;
mod d3d;
mod overlay;
mod pipeline;
mod render;
mod window;

pub(crate) use render::run;
pub(crate) use window::{
    enable_per_monitor_dpi, foreground_window, is_supported, main_window_of, window_info,
};

use crate::error::UpscaleError;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{CreateEventW, SetEvent};

/// Подія Win32 з автоскиданням: будить потік апскейлу (новий кадр, стоп, закриття вікна).
/// Зберігаємо сире значення дескриптора, щоб передавати його в обробники WinRT з інших потоків.
pub(crate) struct Event(isize);

impl Event {
    pub(crate) fn new() -> Result<Self, UpscaleError> {
        // SAFETY: без атрибутів безпеки й імені; дескриптор закривається в Drop.
        let h = unsafe { CreateEventW(None, false, false, None) }.map_err(os("CreateEventW"))?;
        Ok(Self(h.0 as isize))
    }

    pub(crate) fn set(&self) {
        set_raw(self.0);
    }

    fn raw(&self) -> isize {
        self.0
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0 as *mut core::ffi::c_void)
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: дескриптор створений у `new` і більше ніде не закривається.
        let _ = unsafe { CloseHandle(self.handle()) };
    }
}

/// Сигналить подію за сирим дескриптором (з обробників WinRT, що працюють в інших потоках).
/// Дескриптор живе, поки існує [`Event`], а обробники знімаються раніше (див. `capture`).
fn set_raw(raw: isize) {
    // SAFETY: дескриптор події дійсний (див. вище); SetEvent не чіпає нашу пам'ять.
    let _ = unsafe { SetEvent(HANDLE(raw as *mut core::ffi::c_void)) };
}

/// Перетворює помилку WinAPI/COM на [`UpscaleError::Os`] з назвою функції.
fn os(api: &'static str) -> impl Fn(windows::core::Error) -> UpscaleError {
    move |e| UpscaleError::Os {
        api,
        code: e.code().0 as u32,
    }
}
