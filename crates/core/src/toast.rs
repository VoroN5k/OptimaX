//! Короткі повідомлення поверх гри («Запис почнеться за 3 с», «Апскейл увімкнено»).
//!
//! Невелике вікно вгорі по центру монітора, на якому зараз гра: завжди зверху, не забирає
//! фокус, пропускає мишу. Працює поверх віконних і borderless-ігор; поверх ексклюзивного
//! повноекранного режиму Windows вікна не показує — там лишаються звукові сигнали.
//! Тексти передає викликач (з i18n-каталогу).

use crate::error::CoreError;
use std::time::Duration;

/// Тон повідомлення — колір акцентної смужки (з дизайн-токенів).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Нейтрально (`--info`).
    Info,
    /// Успіх (`--ok`).
    Ok,
    /// Увага (`--warn`).
    Warn,
    /// Помилка (`--danger`).
    Danger,
}

/// Скільки показувати повідомлення за замовчуванням.
pub const DEFAULT_DURATION: Duration = Duration::from_secs(2);

/// Показ повідомлень. Власний потік із вікном; зупиняється при знищенні.
pub struct Toaster {
    inner: imp::Toaster,
}

impl Toaster {
    /// Запускає потік повідомлень.
    pub fn new() -> Result<Self, CoreError> {
        Ok(Self {
            inner: imp::Toaster::new()?,
        })
    }

    /// Показує `text` на `duration` (попереднє повідомлення замінюється). Не блокує.
    pub fn show(&self, text: &str, tone: Tone, duration: Duration) {
        self.inner.show(text, tone, duration);
    }
}

/// Розміри повідомлення у фізичних пікселях для монітора заввишки `monitor_height`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Metrics {
    /// Висота шрифту.
    pub font: i32,
    /// Внутрішній відступ.
    pub padding: i32,
    /// Ширина акцентної смужки.
    pub accent: i32,
    /// Радіус скруглення.
    pub radius: i32,
}

impl Metrics {
    pub(crate) fn for_monitor(monitor_height: i32) -> Self {
        // 1080p → 18 px, 1440p → 24 px, 4K → 36 px.
        let font = (monitor_height / 60).clamp(14, 48);
        Self {
            font,
            padding: font * 3 / 4,
            accent: (font / 6).max(3),
            radius: font / 2,
        }
    }

    /// Розмір вікна для тексту розміром `text_w × text_h`.
    pub(crate) fn window_size(self, text_w: i32, text_h: i32) -> (i32, i32) {
        (
            text_w + self.padding * 3 + self.accent,
            text_h + self.padding * 2,
        )
    }
}

/// Лівий верхній кут вікна `size`: по центру монітора, на 6% висоти від верху.
pub(crate) fn position(monitor: (i32, i32, i32, i32), size: (i32, i32)) -> (i32, i32) {
    let (mx, my, mw, mh) = monitor;
    (mx + (mw - size.0) / 2, my + mh * 6 / 100)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use super::{position, Metrics, Tone};
    use crate::error::CoreError;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::JoinHandle;
    use std::time::Duration;
    use windows::core::w;
    use windows::Win32::Foundation::{
        COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
        DeleteDC, DeleteObject, DrawTextW, FillRect, GetDC, GetMonitorInfoW, MonitorFromWindow,
        ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor, AC_SRC_OVER, BLENDFUNCTION,
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_CALCRECT, DT_NOPREFIX,
        DT_SINGLELINE, DT_VCENTER, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
        OUT_DEFAULT_PRECIS, PS_SOLID, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
        GetMessageW, KillTimer, PeekMessageW, PostThreadMessageW, RegisterClassExW, SetTimer,
        SetWindowPos, ShowWindow, TranslateMessage, UpdateLayeredWindow, HTTRANSPARENT,
        HWND_TOPMOST, MA_NOACTIVATE, MSG, PM_NOREMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, ULW_COLORKEY, UPDATE_LAYERED_WINDOW_FLAGS, WM_APP,
        WM_MOUSEACTIVATE, WM_NCHITTEST, WM_QUIT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    const CLASS: windows::core::PCWSTR = w!("OptimaXToast");
    const HIDE_TIMER: usize = 1;
    /// Колір-«ключ»: ним залиті кути за межами скруглення, вони стають прозорими.
    const KEY: COLORREF = rgb(0xFF, 0x00, 0xFF);
    /// Дизайн-токени (CLAUDE.md, 10.2).
    const BG_CARD: COLORREF = rgb(0x12, 0x15, 0x1F);
    const BORDER_STRONG: COLORREF = rgb(0x2A, 0x31, 0x50);
    const TEXT: COLORREF = rgb(0xE8, 0xEA, 0xF2);
    /// Непрозорість усього вікна: повна — крізь картку не просвічує гра, текст читається.
    const ALPHA: u8 = 255;

    const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
    }

    fn accent(tone: Tone) -> COLORREF {
        match tone {
            Tone::Info => rgb(0x6E, 0x7B, 0xFF),
            Tone::Ok => rgb(0x4A, 0xDE, 0x80),
            Tone::Warn => rgb(0xF5, 0xB8, 0x4B),
            Tone::Danger => rgb(0xF0, 0x61, 0x6D),
        }
    }

    struct Command {
        text: String,
        tone: Tone,
        duration: Duration,
    }

    pub(super) struct Toaster {
        thread_id: u32,
        commands: Sender<Command>,
        handle: Option<JoinHandle<()>>,
    }

    impl Toaster {
        pub(super) fn new() -> Result<Self, CoreError> {
            let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, CoreError>>();
            let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
            let handle = std::thread::Builder::new()
                .name("optimax-toast".into())
                .spawn(move || run(&ready_tx, &cmd_rx))
                .map_err(|e| CoreError::InvalidData(e.to_string()))?;
            match ready_rx.recv() {
                Ok(Ok(thread_id)) => Ok(Self {
                    thread_id,
                    commands: cmd_tx,
                    handle: Some(handle),
                }),
                Ok(Err(e)) => {
                    let _ = handle.join();
                    Err(e)
                }
                Err(_) => Err(CoreError::InvalidData(
                    "потік повідомлень не запустився".into(),
                )),
            }
        }

        pub(super) fn show(&self, text: &str, tone: Tone, duration: Duration) {
            let sent = self.commands.send(Command {
                text: text.to_owned(),
                tone,
                duration,
            });
            if sent.is_ok() {
                // SAFETY: потік існує й має чергу (створена до відповіді `ready`).
                let _ = unsafe { PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0)) };
            }
        }
    }

    impl Drop for Toaster {
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
        match msg {
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            // SAFETY: стандартна обробка решти повідомлень з тими самими аргументами.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    fn create_window() -> Result<HWND, CoreError> {
        let os = |api| {
            move |e: windows::core::Error| CoreError::Os {
                api,
                code: e.code().0 as u32,
            }
        };
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
        // SAFETY: структура локальна; повторна реєстрація (другий Toaster) повертає 0 — клас уже є.
        unsafe { RegisterClassExW(&class) };
        // SAFETY: клас зареєстровано; розмір задається при показі.
        unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                CLASS,
                w!("OptimaX"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )
        }
        .map_err(os("CreateWindowExW"))
    }

    fn run(ready: &Sender<Result<u32, CoreError>>, commands: &Receiver<Command>) {
        let mut msg = MSG::default();
        // SAFETY: створює чергу повідомлень потоку до того, як про нього дізнаються інші.
        let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
        let hwnd = match create_window() {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        // SAFETY: без аргументів.
        let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
        loop {
            // SAFETY: `msg` — локальна структура; 0 — WM_QUIT, -1 — помилка.
            let r = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if r <= 0 {
                break;
            }
            match msg.message {
                WM_APP => {
                    // Беремо лише останнє: старіші повідомлення вже неактуальні.
                    if let Some(cmd) = commands.try_iter().last() {
                        show(hwnd, &cmd);
                    }
                }
                WM_TIMER if msg.hwnd == hwnd && msg.wParam.0 == HIDE_TIMER => {
                    // SAFETY: вікно й таймер наші.
                    unsafe {
                        let _ = KillTimer(Some(hwnd), HIDE_TIMER);
                        let _ = ShowWindow(hwnd, SW_HIDE);
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
        // SAFETY: вікно створене цим потоком.
        let _ = unsafe { DestroyWindow(hwnd) };
    }

    /// Монітор вікна на передньому плані (гри): `(x, y, ширина, висота)`.
    fn active_monitor() -> Option<(i32, i32, i32, i32)> {
        // SAFETY: GetForegroundWindow без аргументів; MONITOR_DEFAULTTOPRIMARY завжди дає монітор.
        let monitor = unsafe { MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY) };
        let mut mi = MONITORINFO {
            cbSize: core::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: локальна структура з правильним cbSize.
        if !unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
            return None;
        }
        let r = mi.rcMonitor;
        Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
    }

    fn show(hwnd: HWND, cmd: &Command) {
        let Some(monitor) = active_monitor() else {
            return;
        };
        let metrics = Metrics::for_monitor(monitor.3);
        if draw(hwnd, &cmd.text, cmd.tone, monitor, metrics).is_none() {
            return;
        }
        let ms = u32::try_from(cmd.duration.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: вікно наше; SW_SHOWNOACTIVATE не забирає фокус у гри.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            // Над усім, зокрема над оверлеєм апскейлу.
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            SetTimer(Some(hwnd), HIDE_TIMER, ms, None);
        }
    }

    /// Ресурси GDI, що звільняються в будь-якому разі.
    struct Gdi {
        screen: HDC,
        mem: HDC,
        objects: Vec<HGDIOBJ>,
    }

    impl Drop for Gdi {
        fn drop(&mut self) {
            // SAFETY: контексти й об'єкти створені в `draw` і більше ніде не звільняються;
            // DC видаляємо раніше за об'єкти, що були в нього вибрані.
            unsafe {
                let _ = DeleteDC(self.mem);
                for o in self.objects.drain(..) {
                    let _ = DeleteObject(o);
                }
                ReleaseDC(None, self.screen);
            }
        }
    }

    /// Малює повідомлення в пам'яті й передає вікну через UpdateLayeredWindow.
    fn draw(
        hwnd: HWND,
        text: &str,
        tone: Tone,
        monitor: (i32, i32, i32, i32),
        m: Metrics,
    ) -> Option<()> {
        let mut text16: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: усі виклики GDI — з контекстами й об'єктами, створеними тут і звільненими
        // в Drop для `Gdi`; буфери (`text16`, RECT, POINT, SIZE) локальні.
        unsafe {
            let screen = GetDC(None);
            let mut gdi = Gdi {
                screen,
                mem: CreateCompatibleDC(Some(screen)),
                objects: Vec::new(),
            };
            let font = CreateFontW(
                -m.font,
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            gdi.objects.push(font.into());
            SelectObject(gdi.mem, font.into());

            let mut measure = RECT::default();
            DrawTextW(
                gdi.mem,
                &mut text16,
                &mut measure,
                DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
            );
            let (w, h) = m.window_size(measure.right, measure.bottom);

            let bitmap = CreateCompatibleBitmap(screen, w, h);
            gdi.objects.push(bitmap.into());
            SelectObject(gdi.mem, bitmap.into());

            let key = CreateSolidBrush(KEY);
            let bg = CreateSolidBrush(BG_CARD);
            let bar = CreateSolidBrush(accent(tone));
            let pen = CreatePen(PS_SOLID, 1, BORDER_STRONG);
            gdi.objects
                .extend::<[HGDIOBJ; 4]>([key.into(), bg.into(), bar.into(), pen.into()]);

            let full = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            FillRect(gdi.mem, &full, key);
            SelectObject(gdi.mem, bg.into());
            SelectObject(gdi.mem, pen.into());
            let _ = RoundRect(gdi.mem, 0, 0, w, h, m.radius * 2, m.radius * 2);
            let accent_rect = RECT {
                left: m.padding,
                top: m.padding,
                right: m.padding + m.accent,
                bottom: h - m.padding,
            };
            FillRect(gdi.mem, &accent_rect, bar);

            SetBkMode(gdi.mem, TRANSPARENT);
            SetTextColor(gdi.mem, TEXT);
            let mut text_rect = RECT {
                left: m.padding * 2 + m.accent,
                top: 0,
                right: w - m.padding,
                bottom: h,
            };
            DrawTextW(
                gdi.mem,
                &mut text16,
                &mut text_rect,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );

            let (x, y) = position(monitor, (w, h));
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: ALPHA,
                AlphaFormat: 0,
            };
            UpdateLayeredWindow(
                hwnd,
                Some(screen),
                Some(&POINT { x, y }),
                Some(&SIZE { cx: w, cy: h }),
                Some(gdi.mem),
                Some(&POINT { x: 0, y: 0 }),
                KEY,
                Some(&blend),
                UPDATE_LAYERED_WINDOW_FLAGS(ULW_ALPHA.0 | ULW_COLORKEY.0),
            )
            .ok()
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Tone;
    use crate::error::CoreError;
    use std::time::Duration;

    pub(super) struct Toaster;

    impl Toaster {
        pub(super) fn new() -> Result<Self, CoreError> {
            Err(CoreError::Unsupported("повідомлення поверх гри"))
        }
        pub(super) fn show(&self, _text: &str, _tone: Tone, _duration: Duration) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_scale_with_monitor() {
        assert_eq!(Metrics::for_monitor(1080).font, 18);
        assert_eq!(Metrics::for_monitor(1440).font, 24);
        assert_eq!(Metrics::for_monitor(2160).font, 36);
        // Межі: крихітний і гігантський монітор.
        assert_eq!(Metrics::for_monitor(480).font, 14);
        assert_eq!(Metrics::for_monitor(10_000).font, 48);
        let m = Metrics::for_monitor(1440);
        assert_eq!((m.padding, m.accent, m.radius), (18, 4, 12));
    }

    #[test]
    fn window_fits_text_with_padding() {
        let m = Metrics::for_monitor(1440);
        assert_eq!(m.window_size(300, 32), (300 + 18 * 3 + 4, 32 + 36));
    }

    #[test]
    fn toast_is_centered_near_top_of_its_monitor() {
        assert_eq!(position((0, 0, 2560, 1440), (400, 60)), (1080, 86));
        // Другий монітор зліва від основного.
        assert_eq!(position((-1920, 0, 1920, 1080), (400, 60)), (-1160, 64));
    }
}
