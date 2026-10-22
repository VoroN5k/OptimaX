//! Фокус вікна, глобальна гаряча клавіша та звукові сигнали — для старту запису «з гри».

use crate::error::CoreError;
use std::sync::mpsc::Receiver;

/// PID процесу, чиє вікно зараз на передньому плані (`None`, якщо визначити не вдалося).
pub fn foreground_process_id() -> Option<u32> {
    imp::foreground_process_id()
}

/// Сигнал «запис почався» (вищий тон). Не блокує: грає в окремому потоці.
pub fn beep_start() {
    imp::beep(880, 110);
}

/// Сигнал «запис завершено» (два нижчі тони).
pub fn beep_stop() {
    imp::beep_twice(660, 90);
}

/// Глобальна гаряча клавіша `Ctrl+Shift+F9`. Працює й поверх повноекранних ігор.
/// Натискання приходять у канал [`GlobalHotkey::presses`]; знімається при знищенні.
pub struct GlobalHotkey {
    inner: imp::Hotkey,
    presses: Receiver<()>,
}

impl GlobalHotkey {
    /// Реєструє `Ctrl+Shift+F9`. Помилка, якщо клавішу вже зайняла інша програма.
    pub fn ctrl_shift_f9() -> Result<Self, CoreError> {
        let (inner, presses) = imp::Hotkey::register()?;
        Ok(Self { inner, presses })
    }

    /// Чи було натискання з минулої перевірки (усі накопичені натискання зливаються в одне).
    pub fn pressed(&self) -> bool {
        let mut any = false;
        while self.presses.try_recv().is_ok() {
            any = true;
        }
        any
    }
}

impl Drop for GlobalHotkey {
    fn drop(&mut self) {
        self.inner.shutdown();
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use crate::error::CoreError;
    use std::sync::mpsc::{self, Receiver};
    use std::thread::JoinHandle;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Diagnostics::Debug::Beep;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_F9,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, PeekMessageW,
        PostThreadMessageW, MSG, PM_NOREMOVE, WM_HOTKEY, WM_QUIT,
    };

    const HOTKEY_ID: i32 = 0x0F9;

    pub(super) fn foreground_process_id() -> Option<u32> {
        // SAFETY: функції без вказівників на наші дані, крім локального `pid`.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        // SAFETY: `hwnd` отримано щойно; `pid` — локальна змінна.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        (pid != 0).then_some(pid)
    }

    pub(super) fn beep(freq: u32, ms: u32) {
        let _ = std::thread::Builder::new()
            .name("optimax-beep".into())
            // SAFETY: звичайний системний виклик зі значеннями.
            .spawn(move || unsafe {
                let _ = Beep(freq, ms);
            });
    }

    pub(super) fn beep_twice(freq: u32, ms: u32) {
        let _ = std::thread::Builder::new()
            .name("optimax-beep".into())
            // SAFETY: як вище.
            .spawn(move || unsafe {
                let _ = Beep(freq, ms);
                std::thread::sleep(std::time::Duration::from_millis(60));
                let _ = Beep(freq * 3 / 4, ms);
            });
    }

    pub(super) struct Hotkey {
        thread_id: u32,
        handle: Option<JoinHandle<()>>,
    }

    impl Hotkey {
        /// Окремий потік із чергою повідомлень: RegisterHotKey прив'язує клавішу до потоку,
        /// який її зареєстрував, і WM_HOTKEY приходить саме в його чергу.
        pub(super) fn register() -> Result<(Self, Receiver<()>), CoreError> {
            let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, CoreError>>();
            let (press_tx, press_rx) = mpsc::channel::<()>();
            let handle = std::thread::Builder::new()
                .name("optimax-hotkey".into())
                .spawn(move || {
                    let mut msg = MSG::default();
                    // SAFETY: створює чергу повідомлень потоку (до PostThreadMessage ззовні).
                    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
                    // SAFETY: прив'язка до потоку (hwnd = None), id — наша константа.
                    let reg = unsafe {
                        RegisterHotKey(
                            None,
                            HOTKEY_ID,
                            MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                            u32::from(VK_F9.0),
                        )
                    };
                    if let Err(e) = reg {
                        let _ = ready_tx.send(Err(CoreError::Os {
                            api: "RegisterHotKey",
                            code: (e.code().0 as u32) & 0xFFFF,
                        }));
                        return;
                    }
                    // SAFETY: без аргументів.
                    let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
                    loop {
                        // SAFETY: `msg` — локальна структура; 0 — отримано WM_QUIT, -1 — помилка.
                        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
                        if r <= 0 || msg.message == WM_QUIT {
                            break;
                        }
                        if msg.message == WM_HOTKEY && press_tx.send(()).is_err() {
                            break;
                        }
                    }
                    // SAFETY: знімаємо реєстрацію в тому самому потоці.
                    let _ = unsafe { UnregisterHotKey(None, HOTKEY_ID) };
                })
                .map_err(|e| CoreError::InvalidData(e.to_string()))?;
            match ready_rx.recv() {
                Ok(Ok(thread_id)) => Ok((
                    Self {
                        thread_id,
                        handle: Some(handle),
                    },
                    press_rx,
                )),
                Ok(Err(e)) => {
                    let _ = handle.join();
                    Err(e)
                }
                Err(_) => Err(CoreError::InvalidData(
                    "потік гарячої клавіші не запустився".into(),
                )),
            }
        }

        pub(super) fn shutdown(&mut self) {
            if let Some(h) = self.handle.take() {
                // SAFETY: потік існує й має чергу (створена PeekMessage перед реєстрацією).
                let _ =
                    unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
                let _ = h.join();
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::error::CoreError;
    use std::sync::mpsc::Receiver;

    pub(super) fn foreground_process_id() -> Option<u32> {
        None
    }
    pub(super) fn beep(_freq: u32, _ms: u32) {}
    pub(super) fn beep_twice(_freq: u32, _ms: u32) {}

    pub(super) struct Hotkey;

    impl Hotkey {
        pub(super) fn register() -> Result<(Self, Receiver<()>), CoreError> {
            Err(CoreError::Unsupported("глобальні гарячі клавіші"))
        }
        pub(super) fn shutdown(&mut self) {}
    }
}
