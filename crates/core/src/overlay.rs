//! Оверлей у грі: кілька рядків (FPS, 1% low, CPU/GPU) у куті монітора, на якому гра.
//!
//! Те саме, що й [`crate::toast`]: вікно GDI «завжди зверху», не забирає фокус, пропускає мишу,
//! малюється в пам'яті й передається через `UpdateLayeredWindow` — без власного ланцюжка обміну
//! на GPU. Оновлюється рідко (викликач — двічі на секунду). Працює поверх віконних і
//! borderless-ігор; поверх ексклюзивного повноекранного режиму Windows вікна не показує.
//! Тексти передає викликач (з i18n-каталогу).

use crate::error::CoreError;

/// Колір рядка (з дизайн-токенів).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineTone {
    /// Основний текст (`--text`).
    Normal,
    /// Допоміжний (`--text-muted`).
    Muted,
    /// Позначка «запис іде» (`--danger`).
    Danger,
}

/// Рядок оверлею.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayLine {
    /// Текст.
    pub text: String,
    /// Колір.
    pub tone: LineTone,
}

impl OverlayLine {
    /// Рядок з основним кольором.
    pub fn normal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: LineTone::Normal,
        }
    }

    /// Рядок допоміжним кольором.
    pub fn muted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: LineTone::Muted,
        }
    }
}

/// Оверлей. Власний потік із вікном; зупиняється при знищенні.
pub struct Overlay {
    inner: imp::Overlay,
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Overlay")
    }
}

impl Overlay {
    /// Створює (приховане) вікно оверлею.
    pub fn new() -> Result<Self, CoreError> {
        Ok(Self {
            inner: imp::Overlay::new()?,
        })
    }

    /// Показує рядки (замінює попередні). Не блокує.
    pub fn show(&self, lines: Vec<OverlayLine>) {
        self.inner.send(Some(lines));
    }

    /// Ховає оверлей. Не блокує.
    pub fn hide(&self) {
        self.inner.send(None);
    }
}

/// Розміри у фізичних пікселях для монітора заввишки `monitor_height`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Metrics {
    /// Висота шрифту.
    pub font: i32,
    /// Внутрішній відступ.
    pub padding: i32,
    /// Відступ вікна від краю монітора.
    pub margin: i32,
    /// Радіус скруглення.
    pub radius: i32,
}

impl Metrics {
    pub(crate) fn for_monitor(monitor_height: i32) -> Self {
        // 1080p → 15 px, 1440p → 20 px, 4K → 30 px: дрібніше за повідомлення — оверлей постійний.
        let font = (monitor_height / 72).clamp(12, 40);
        Self {
            font,
            padding: font / 2,
            margin: font,
            radius: font / 3,
        }
    }

    /// Розмір вікна для рядків розмірами `(ширина, висота)`.
    pub(crate) fn window_size(self, lines: &[(i32, i32)]) -> (i32, i32) {
        let w = lines.iter().map(|l| l.0).max().unwrap_or(0);
        let h: i32 = lines.iter().map(|l| l.1).sum();
        (w + self.padding * 2, h + self.padding * 2)
    }

    /// Лівий верхній кут вікна: лівий верхній кут монітора з відступом.
    pub(crate) fn position(self, monitor: (i32, i32, i32, i32)) -> (i32, i32) {
        (monitor.0 + self.margin, monitor.1 + self.margin)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use super::{LineTone, Metrics, OverlayLine};
    use crate::error::CoreError;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::JoinHandle;
    use windows::core::w;
    use windows::Win32::Foundation::{
        COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
        DeleteDC, DeleteObject, DrawTextW, FillRect, GetDC, GetMonitorInfoW, MonitorFromWindow,
        ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor, AC_SRC_OVER, BLENDFUNCTION,
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_CALCRECT, DT_NOPREFIX,
        DT_SINGLELINE, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, OUT_DEFAULT_PRECIS,
        PS_SOLID, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
        GetMessageW, PeekMessageW, PostThreadMessageW, RegisterClassExW, SetWindowPos, ShowWindow,
        TranslateMessage, UpdateLayeredWindow, HTTRANSPARENT, HWND_TOPMOST, MA_NOACTIVATE, MSG,
        PM_NOREMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA,
        ULW_COLORKEY, UPDATE_LAYERED_WINDOW_FLAGS, WM_APP, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_QUIT,
        WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_EX_TRANSPARENT, WS_POPUP,
    };

    const CLASS: windows::core::PCWSTR = w!("OptimaXOverlay");
    /// Колір-«ключ»: ним залиті кути за межами скруглення, вони стають прозорими.
    const KEY: COLORREF = rgb(0xFF, 0x00, 0xFF);
    /// Дизайн-токени (CLAUDE.md, 10.2).
    const BG_CARD: COLORREF = rgb(0x12, 0x15, 0x1F);
    const BORDER: COLORREF = rgb(0x1D, 0x22, 0x33);
    const TEXT: COLORREF = rgb(0xE8, 0xEA, 0xF2);
    const TEXT_MUTED: COLORREF = rgb(0x8A, 0x90, 0xA8);
    const DANGER: COLORREF = rgb(0xF0, 0x61, 0x6D);
    /// Непрозорість: гра трохи просвічує, текст читається.
    const ALPHA: u8 = 225;

    const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
    }

    fn color(t: LineTone) -> COLORREF {
        match t {
            LineTone::Normal => TEXT,
            LineTone::Muted => TEXT_MUTED,
            LineTone::Danger => DANGER,
        }
    }

    pub(super) struct Overlay {
        thread_id: u32,
        commands: Sender<Option<Vec<OverlayLine>>>,
        handle: Option<JoinHandle<()>>,
    }

    impl Overlay {
        pub(super) fn new() -> Result<Self, CoreError> {
            let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, CoreError>>();
            let (cmd_tx, cmd_rx) = mpsc::channel();
            let handle = std::thread::Builder::new()
                .name("optimax-overlay".into())
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
                Err(_) => Err(CoreError::InvalidData("потік оверлею не запустився".into())),
            }
        }

        pub(super) fn send(&self, cmd: Option<Vec<OverlayLine>>) {
            if self.commands.send(cmd).is_ok() {
                // SAFETY: потік існує й має чергу (створена до відповіді `ready`).
                let _ = unsafe { PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0)) };
            }
        }
    }

    impl Drop for Overlay {
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
        // SAFETY: структура локальна; повторна реєстрація повертає 0 — клас уже є.
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
                w!("OptimaX overlay"),
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

    fn run(ready: &Sender<Result<u32, CoreError>>, commands: &Receiver<Option<Vec<OverlayLine>>>) {
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
        let mut visible = false;
        loop {
            // SAFETY: `msg` — локальна структура; 0 — WM_QUIT, -1 — помилка.
            let r = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if r <= 0 {
                break;
            }
            if msg.message == WM_APP {
                // Беремо лише останню команду: старіші вже неактуальні.
                match commands.try_iter().last() {
                    Some(Some(lines)) => visible = show(hwnd, &lines, visible),
                    Some(None) if visible => {
                        // SAFETY: вікно наше.
                        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
                        visible = false;
                    }
                    _ => {}
                }
                continue;
            }
            // SAFETY: звичайна обробка повідомлень цього потоку.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
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

    /// Малює й показує; повертає, чи видно вікно.
    fn show(hwnd: HWND, lines: &[OverlayLine], visible: bool) -> bool {
        let Some(monitor) = active_monitor() else {
            return visible;
        };
        if lines.is_empty() || draw(hwnd, lines, monitor, Metrics::for_monitor(monitor.3)).is_none()
        {
            return visible;
        }
        // SAFETY: вікно наше; SW_SHOWNOACTIVATE не забирає фокус у гри. TOPMOST щоразу:
        // гра в borderless могла стати вище за нас.
        unsafe {
            if !visible {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        true
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

    /// Малює рядки в пам'яті й передає вікну через UpdateLayeredWindow.
    fn draw(
        hwnd: HWND,
        lines: &[OverlayLine],
        monitor: (i32, i32, i32, i32),
        m: Metrics,
    ) -> Option<()> {
        let mut texts: Vec<Vec<u16>> = lines
            .iter()
            .map(|l| l.text.encode_utf16().collect())
            .collect();
        // SAFETY: усі виклики GDI — з контекстами й об'єктами, створеними тут і звільненими
        // в Drop для `Gdi`; буфери (`texts`, RECT, POINT, SIZE) локальні.
        unsafe {
            let screen = GetDC(None);
            let mut gdi = Gdi {
                screen,
                mem: CreateCompatibleDC(Some(screen)),
                objects: Vec::new(),
            };
            // Моноширинний шрифт: цифри, що змінюються, не «стрибають» по ширині.
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
                w!("Consolas"),
            );
            gdi.objects.push(font.into());
            SelectObject(gdi.mem, font.into());

            let sizes: Vec<(i32, i32)> = texts
                .iter_mut()
                .map(|t| {
                    let mut r = RECT::default();
                    DrawTextW(
                        gdi.mem,
                        t,
                        &mut r,
                        DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
                    );
                    (r.right, r.bottom)
                })
                .collect();
            let (w, h) = m.window_size(&sizes);

            let bitmap = CreateCompatibleBitmap(screen, w, h);
            gdi.objects.push(bitmap.into());
            SelectObject(gdi.mem, bitmap.into());

            let key = CreateSolidBrush(KEY);
            let bg = CreateSolidBrush(BG_CARD);
            let pen = CreatePen(PS_SOLID, 1, BORDER);
            gdi.objects
                .extend::<[HGDIOBJ; 3]>([key.into(), bg.into(), pen.into()]);
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

            SetBkMode(gdi.mem, TRANSPARENT);
            let mut y = m.padding;
            for ((text, line), (_, lh)) in texts.iter_mut().zip(lines).zip(&sizes) {
                SetTextColor(gdi.mem, color(line.tone));
                let mut r = RECT {
                    left: m.padding,
                    top: y,
                    right: w - m.padding,
                    bottom: y + lh,
                };
                DrawTextW(gdi.mem, text, &mut r, DT_SINGLELINE | DT_NOPREFIX);
                y += lh;
            }

            let (x, y) = m.position(monitor);
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
    use super::OverlayLine;
    use crate::error::CoreError;

    pub(super) struct Overlay;

    impl Overlay {
        pub(super) fn new() -> Result<Self, CoreError> {
            Err(CoreError::Unsupported("оверлей у грі"))
        }
        pub(super) fn send(&self, _cmd: Option<Vec<OverlayLine>>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_scale_with_monitor() {
        assert_eq!(Metrics::for_monitor(1080).font, 15);
        assert_eq!(Metrics::for_monitor(1440).font, 20);
        assert_eq!(Metrics::for_monitor(2160).font, 30);
        assert_eq!(Metrics::for_monitor(480).font, 12);
        assert_eq!(Metrics::for_monitor(10_000).font, 40);
    }

    #[test]
    fn window_fits_widest_line_and_all_heights() {
        let m = Metrics::for_monitor(1440);
        assert_eq!(
            m.window_size(&[(120, 24), (200, 24), (80, 24)]),
            (200 + 20, 72 + 20)
        );
        assert_eq!(m.window_size(&[]), (20, 20));
    }

    #[test]
    fn overlay_sits_in_top_left_of_its_monitor() {
        let m = Metrics::for_monitor(1440);
        assert_eq!(m.position((0, 0, 2560, 1440)), (20, 20));
        // Другий монітор зліва від основного.
        assert_eq!(
            Metrics::for_monitor(1080).position((-1920, 0, 1920, 1080)),
            (-1905, 15)
        );
    }
}
