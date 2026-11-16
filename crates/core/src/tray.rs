//! Іконка в області сповіщень (трей) з контекстним меню — для агента OptimaX (етап 4
//! `docs/SERVICE.md`). Власна обгортка над `Shell_NotifyIconW`: окремий потік із прихованим
//! вікном, як у [`crate::toast`]. Тексти передає викликач (з i18n-каталогу).
//!
//! Лівий клік — [`TrayEvent::Activate`], правий — меню; вибраний пункт — [`TrayEvent::Command`].
//! Після перезапуску Провідника (`TaskbarCreated`) іконка додається знову.

use crate::error::CoreError;
use std::sync::mpsc::Sender;

/// Пункт меню.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    /// Звичайний пункт.
    Item {
        /// Що повернеться в [`TrayEvent::Command`] (не 0).
        id: u32,
        /// Текст.
        label: String,
        /// Позначка-галочка.
        checked: bool,
        /// Можна вибрати.
        enabled: bool,
        /// Жирний пункт за замовчуванням.
        default: bool,
    },
    /// Роздільник.
    Separator,
}

impl MenuItem {
    /// Звичайний доступний пункт.
    pub fn item(id: u32, label: impl Into<String>) -> Self {
        Self::Item {
            id,
            label: label.into(),
            checked: false,
            enabled: true,
            default: false,
        }
    }
}

/// Що зробив користувач.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Лівий клік по іконці.
    Activate,
    /// Вибрано пункт меню з цим `id`.
    Command(u32),
}

/// Іконка в треї. Зникає, коли значення знищується.
pub struct Tray {
    inner: imp::Tray,
}

impl Tray {
    /// Показує іконку з підказкою `tooltip` і меню `menu`; події йдуть у `events`.
    pub fn new(
        tooltip: &str,
        menu: Vec<MenuItem>,
        events: Sender<TrayEvent>,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            inner: imp::Tray::new(tooltip, menu, events)?,
        })
    }

    /// Змінює підказку й меню (наступне відкриття меню покаже нове).
    pub fn update(&self, tooltip: &str, menu: Vec<MenuItem>) {
        self.inner.update(tooltip, menu);
    }
}

/// Підказка для `NOTIFYICONDATAW::szTip`: щонайбільше 127 одиниць UTF-16 і нуль у кінці,
/// без розрізаної сурогатної пари.
pub(crate) fn tip_utf16(text: &str) -> [u16; 128] {
    let mut out = [0u16; 128];
    let mut n = 0;
    for c in text.chars() {
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        if n + units.len() > 127 {
            break;
        }
        out[n..n + units.len()].copy_from_slice(units);
        n += units.len();
    }
    out
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use super::{tip_utf16, MenuItem, TrayEvent};
    use crate::error::CoreError;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::thread::JoinHandle;
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
        DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, LoadIconW,
        LoadImageW, PeekMessageW, PostMessageW, PostThreadMessageW, RegisterClassExW,
        RegisterWindowMessageW, SetForegroundWindow, SetMenuDefaultItem, TrackPopupMenu,
        TranslateMessage, HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, MF_CHECKED,
        MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PM_NOREMOVE, SM_CXSMICON, SM_CYSMICON,
        TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP,
        WM_NULL, WM_QUIT, WM_RBUTTONUP, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    const CLASS: PCWSTR = w!("OptimaXTray");
    const ICON_ID: u32 = 1;
    /// Підказку чи меню змінено (повідомлення потоку).
    const WM_REFRESH: u32 = WM_APP;
    /// Повідомлення від іконки вікну.
    const WM_TRAY: u32 = WM_APP + 1;
    /// Те саме, переслане в чергу потоку.
    const WM_TRAY_THREAD: u32 = WM_APP + 2;
    /// Провідник перезапустився — іконку треба додати знову.
    const WM_RECREATE: u32 = WM_APP + 3;

    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

    struct Shared {
        tooltip: String,
        menu: Vec<MenuItem>,
    }

    pub(super) struct Tray {
        thread_id: u32,
        shared: Arc<Mutex<Shared>>,
        handle: Option<JoinHandle<()>>,
    }

    impl Tray {
        pub(super) fn new(
            tooltip: &str,
            menu: Vec<MenuItem>,
            events: Sender<TrayEvent>,
        ) -> Result<Self, CoreError> {
            let shared = Arc::new(Mutex::new(Shared {
                tooltip: tooltip.to_owned(),
                menu,
            }));
            let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, CoreError>>();
            let handle = {
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name("optimax-tray".into())
                    .spawn(move || run(&ready_tx, &shared, &events))
                    .map_err(|e| CoreError::InvalidData(e.to_string()))?
            };
            match ready_rx.recv() {
                Ok(Ok(thread_id)) => Ok(Self {
                    thread_id,
                    shared,
                    handle: Some(handle),
                }),
                Ok(Err(e)) => {
                    let _ = handle.join();
                    Err(e)
                }
                Err(_) => Err(CoreError::InvalidData("потік трею не запустився".into())),
            }
        }

        pub(super) fn update(&self, tooltip: &str, menu: Vec<MenuItem>) {
            {
                let mut s = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
                s.tooltip = tooltip.to_owned();
                s.menu = menu;
            }
            // SAFETY: потік існує й має чергу (створена до відповіді `ready`).
            let _ = unsafe { PostThreadMessageW(self.thread_id, WM_REFRESH, WPARAM(0), LPARAM(0)) };
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            if let Some(h) = self.handle.take() {
                // SAFETY: як вище.
                let _ =
                    unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
                let _ = h.join();
            }
        }
    }

    extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let created = TASKBAR_CREATED.load(Ordering::Relaxed);
        let forward = if msg == WM_TRAY {
            Some(WM_TRAY_THREAD)
        } else if created != 0 && msg == created {
            Some(WM_RECREATE)
        } else {
            None
        };
        if let Some(to) = forward {
            // SAFETY: черга цього самого потоку (вікно створене ним).
            let _ = unsafe { PostThreadMessageW(GetCurrentThreadId(), to, wparam, lparam) };
            return LRESULT(0);
        }
        // SAFETY: стандартна обробка решти повідомлень з тими самими аргументами.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn os(api: &'static str) -> impl Fn(windows::core::Error) -> CoreError {
        move |e| CoreError::Os {
            api,
            code: e.code().0 as u32,
        }
    }

    fn create_window() -> Result<(HWND, HINSTANCE), CoreError> {
        // SAFETY: модуль поточного процесу.
        let module = unsafe { GetModuleHandleW(None) }.map_err(os("GetModuleHandleW"))?;
        let instance = HINSTANCE(module.0);
        let class = WNDCLASSEXW {
            cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: CLASS,
            ..Default::default()
        };
        // SAFETY: структура локальна; повторна реєстрація повертає 0 — клас уже є.
        unsafe { RegisterClassExW(&class) };
        // SAFETY: клас зареєстровано; вікно ніколи не показується (лише приймає повідомлення).
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                CLASS,
                w!("OptimaX"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )
        }
        .map_err(os("CreateWindowExW"))?;
        Ok((hwnd, instance))
    }

    /// Мала іконка з ресурсу exe (та сама, що й у вікна); немає — стандартна.
    fn load_icon(instance: HINSTANCE) -> (HICON, bool) {
        // SAFETY: MAKEINTRESOURCE — числовий ідентифікатор як вказівник; розміри — системні.
        let own = unsafe {
            LoadImageW(
                Some(instance),
                PCWSTR(usize::from(crate::taskbar::APP_ICON_RESOURCE_ID) as *const u16),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            )
        };
        match own {
            Ok(h) => (HICON(h.0), true),
            // SAFETY: системна іконка; звільняти не треба.
            Err(_) => (
                unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default(),
                false,
            ),
        }
    }

    fn notify_data(hwnd: HWND, icon: HICON, tooltip: &str) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: core::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            szTip: tip_utf16(tooltip),
            ..Default::default()
        }
    }

    fn tooltip_of(shared: &Mutex<Shared>) -> String {
        shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .tooltip
            .clone()
    }

    fn run(
        ready: &Sender<Result<u32, CoreError>>,
        shared: &Mutex<Shared>,
        events: &Sender<TrayEvent>,
    ) {
        let mut msg = MSG::default();
        // SAFETY: створює чергу повідомлень потоку до того, як про нього дізнаються інші.
        let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
        let (hwnd, instance) = match create_window() {
            Ok(w) => w,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        // SAFETY: рядок-константа.
        let created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        TASKBAR_CREATED.store(created, Ordering::Relaxed);
        let (icon, own_icon) = load_icon(instance);
        let data = notify_data(hwnd, icon, &tooltip_of(shared));
        // SAFETY: структура локальна з правильним cbSize.
        if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            // SAFETY: вікно створене цим потоком.
            let _ = unsafe { DestroyWindow(hwnd) };
            let _ = ready.send(Err(CoreError::Os {
                api: "Shell_NotifyIconW",
                code: 0,
            }));
            return;
        }
        // SAFETY: без аргументів.
        let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
        loop {
            // SAFETY: `msg` — локальна структура; 0 — WM_QUIT, -1 — помилка.
            let r = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if r <= 0 {
                break;
            }
            match msg.message {
                WM_REFRESH => {
                    let data = notify_data(hwnd, icon, &tooltip_of(shared));
                    // SAFETY: як вище.
                    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
                }
                WM_RECREATE => {
                    let data = notify_data(hwnd, icon, &tooltip_of(shared));
                    // SAFETY: як вище.
                    let _ = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
                }
                WM_TRAY_THREAD => {
                    // Без NOTIFYICON_VERSION_4: у lParam — саме повідомлення миші.
                    let mouse = (msg.lParam.0 as u32) & 0xFFFF;
                    if mouse == WM_LBUTTONUP {
                        let _ = events.send(TrayEvent::Activate);
                    } else if mouse == WM_RBUTTONUP || mouse == WM_CONTEXTMENU {
                        let menu = shared
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .menu
                            .clone();
                        if let Some(id) = show_menu(hwnd, &menu) {
                            let _ = events.send(TrayEvent::Command(id));
                        }
                    }
                }
                _ => {
                    // SAFETY: звичайна обробка повідомлень цього потоку.
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
        }
        let data = notify_data(hwnd, icon, "");
        // SAFETY: іконку й вікно створив цей потік; іконку з ресурсу exe звільняємо, системну — ні.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            if own_icon {
                let _ = DestroyIcon(icon);
            }
            let _ = DestroyWindow(hwnd);
        }
    }

    /// Показує меню біля курсора; вибраний пункт або `None`.
    fn show_menu(hwnd: HWND, items: &[MenuItem]) -> Option<u32> {
        // SAFETY: меню створюється й знищується тут; рядки (HSTRING) живі на час AppendMenuW,
        // який копіює текст; вікно — наше.
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            for item in items {
                match item {
                    MenuItem::Separator => {
                        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                    }
                    MenuItem::Item {
                        id,
                        label,
                        checked,
                        enabled,
                        default,
                    } => {
                        let mut flags = MF_STRING;
                        if *checked {
                            flags |= MF_CHECKED;
                        }
                        if !*enabled {
                            flags |= MF_GRAYED;
                        }
                        let _ =
                            AppendMenuW(menu, flags, *id as usize, &HSTRING::from(label.as_str()));
                        if *default {
                            let _ = SetMenuDefaultItem(menu, *id, 0);
                        }
                    }
                }
            }
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // Без цього меню не закривається кліком повз нього (документована вимога).
            let _ = SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                None,
                hwnd,
                None,
            )
            .0;
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            u32::try_from(chosen).ok().filter(|&id| id != 0)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{MenuItem, TrayEvent};
    use crate::error::CoreError;
    use std::sync::mpsc::Sender;

    pub(super) struct Tray;

    impl Tray {
        pub(super) fn new(
            _tooltip: &str,
            _menu: Vec<MenuItem>,
            _events: Sender<TrayEvent>,
        ) -> Result<Self, CoreError> {
            Err(CoreError::Unsupported("іконка в треї"))
        }
        pub(super) fn update(&self, _tooltip: &str, _menu: Vec<MenuItem>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_is_truncated_without_breaking_characters() {
        let t = tip_utf16("OptimaX — агент");
        let n = t.iter().position(|&u| u == 0).unwrap();
        assert_eq!(String::from_utf16(&t[..n]).unwrap(), "OptimaX — агент");
        // Довгий текст: не більше 127 одиниць і нуль у кінці.
        let long = "я".repeat(300);
        let t = tip_utf16(&long);
        assert_eq!(t[127], 0);
        assert_eq!(t.iter().position(|&u| u == 0), Some(127));
        // Емодзі (сурогатна пара) на межі не розрізається.
        let edge = format!("{}😀", "a".repeat(126));
        let t = tip_utf16(&edge);
        assert_eq!(t.iter().position(|&u| u == 0), Some(126));
    }
}
