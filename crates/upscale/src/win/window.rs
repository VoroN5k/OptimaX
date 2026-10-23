//! Геометрія вікон і моніторів, вікно на передньому плані.

use crate::geometry::Rect;
use crate::window::{WindowId, WindowInfo};
use core::ffi::c_void;
use windows::core::BOOL;
use windows::core::PWSTR;
use windows::Graphics::Capture::GraphicsCaptureSession;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetClientRect, GetForegroundWindow, GetWindow, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, GA_ROOT, GW_OWNER,
};

/// Де вікно зараз (фізичні пікселі, координати робочого столу).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Geometry {
    /// Видимі межі вікна (саме їх захоплює WGC).
    pub frame: Rect,
    /// Клієнтська область.
    pub client: Rect,
    /// Монітор вікна.
    pub monitor: Rect,
    /// Згорнуте.
    pub minimized: bool,
}

pub(super) fn hwnd(id: WindowId) -> HWND {
    HWND(id.0 as *mut c_void)
}

fn rect(r: RECT) -> Rect {
    Rect::from_edges(r.left, r.top, r.right, r.bottom)
}

/// Геометрія вікна (`None`, якщо вікна вже немає).
pub(super) fn geometry(h: HWND) -> Option<Geometry> {
    // SAFETY: IsWindow приймає будь-яке значення HWND.
    if !unsafe { IsWindow(Some(h)) }.as_bool() {
        return None;
    }
    // SAFETY: `h` перевірено вище.
    let minimized = unsafe { IsIconic(h) }.as_bool();

    let mut frame = RECT::default();
    // SAFETY: буфер — локальний RECT потрібного розміру.
    unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut frame as *mut RECT).cast::<c_void>(),
            core::mem::size_of::<RECT>() as u32,
        )
    }
    .ok()?;

    let mut client = RECT::default();
    // SAFETY: локальний RECT.
    unsafe { GetClientRect(h, &mut client) }.ok()?;
    let mut origin = POINT::default();
    // SAFETY: локальна POINT.
    if !unsafe { ClientToScreen(h, &mut origin) }.as_bool() {
        return None;
    }

    // SAFETY: MONITOR_DEFAULTTONEAREST завжди повертає монітор.
    let monitor = unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) };
    let mut mi = MONITORINFO {
        cbSize: core::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `mi` — локальна структура з правильним cbSize.
    if !unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
        return None;
    }

    Some(Geometry {
        frame: rect(frame),
        client: Rect::from_edges(
            origin.x,
            origin.y,
            origin.x.saturating_add(client.right - client.left),
            origin.y.saturating_add(client.bottom - client.top),
        ),
        monitor: rect(mi.rcMonitor),
        minimized,
    })
}

/// Корінь вікна на передньому плані.
pub(super) fn foreground_root() -> Option<HWND> {
    // SAFETY: без аргументів.
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() {
        return None;
    }
    // SAFETY: `fg` щойно отримано; для неіснуючого вікна повертається нульовий HWND.
    let root = unsafe { GetAncestor(fg, GA_ROOT) };
    Some(if root.is_invalid() { fg } else { root })
}

/// Ім'я exe процесу `pid` (без шляху); порожнє, якщо процес недоступний.
fn exe_name(pid: u32) -> String {
    // SAFETY: лише право читання імені; дескриптор закривається нижче.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return String::new();
    };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: буфер локальний, `len` — його довжина в символах.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    }
    .is_ok();
    // SAFETY: дескриптор відкрито вище.
    let _ = unsafe { CloseHandle(process) };
    if !ok {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]);
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn info(h: HWND) -> Option<WindowInfo> {
    let g = geometry(h)?;
    let mut pid = 0u32;
    // SAFETY: `pid` — локальна змінна.
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    let mut buf = [0u16; 512];
    // SAFETY: буфер локальний, довжину передає обгортка.
    let len = unsafe { GetWindowTextW(h, &mut buf) };
    let len = usize::try_from(len).unwrap_or(0).min(buf.len());
    Some(WindowInfo {
        id: WindowId(h.0 as isize),
        pid,
        exe: exe_name(pid),
        title: String::from_utf16_lossy(&buf[..len]),
        client: g.client.size(),
        monitor: g.monitor,
        minimized: g.minimized,
    })
}

pub(crate) fn foreground_window() -> Option<WindowInfo> {
    info(foreground_root()?)
}

pub(crate) fn window_info(id: WindowId) -> Option<WindowInfo> {
    info(hwnd(id))
}

/// Пошук головного вікна процесу для [`EnumWindows`].
struct Search {
    pid: u32,
    best: Option<(u64, HWND)>,
}

extern "system" fn consider(h: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` — вказівник на `Search` у `main_window_of`, живий на час EnumWindows,
    // і зворотний виклик відбувається в тому самому потоці.
    let search = unsafe { &mut *(lparam.0 as *mut Search) };
    let mut pid = 0u32;
    // SAFETY: `h` надає EnumWindows; `pid` — локальна змінна.
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    // SAFETY: `h` надає EnumWindows.
    let visible = unsafe { IsWindowVisible(h) }.as_bool();
    // Вікна з власником — діалоги й спливні вікна, не головне вікно гри.
    // SAFETY: як вище; помилка означає «власника немає».
    let owned = unsafe { GetWindow(h, GW_OWNER) }.is_ok_and(|o| !o.is_invalid());
    if pid == search.pid && visible && !owned {
        if let Some(g) = geometry(h) {
            let area = u64::from(g.client.width) * u64::from(g.client.height);
            if area > 0 && search.best.is_none_or(|(a, _)| area > a) {
                search.best = Some((area, h));
            }
        }
    }
    true.into()
}

/// Найбільше видиме вікно верхнього рівня процесу `pid` без власника.
pub(crate) fn main_window_of(pid: u32) -> Option<WindowInfo> {
    let mut search = Search { pid, best: None };
    // SAFETY: `search` живе до кінця виклику; `consider` не зберігає вказівник.
    let _ = unsafe { EnumWindows(Some(consider), LPARAM(&mut search as *mut Search as isize)) };
    info(search.best?.1)
}

pub(crate) fn is_supported() -> bool {
    GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

pub(crate) fn enable_per_monitor_dpi() {
    // SAFETY: системна константа; повторний виклик просто повертає помилку, яку ігноруємо.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}
