//! Сеанс апскейлу: окремий потік із власним вікном-оверлеєм, захопленням і рендером.
//! Зупиняється викликом [`Upscaler::stop`], знищенням або сам — коли вікно гри закрили.

use crate::error::UpscaleError;
use crate::geometry::{Rect, Size};
use crate::imp;
use crate::settings::UpscaleSettings;
use crate::window::WindowId;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Скільки чекати, поки потік підготує захоплення й оверлей.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// Чому сеанс завершився.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum EndReason {
    /// Зупинено користувачем.
    Stopped,
    /// Вікно гри закрили.
    WindowClosed,
    /// Збій під час роботи (напр. скинувся відеодрайвер).
    Failed(String),
}

/// Лічильники сеансу (для статусу в UI і вимірювань).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpscaleStats {
    /// Кадрів отримано від захоплення.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub captured: u64,
    /// Кадрів показано в оверлеї.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub presented: u64,
    /// Розмір кадру гри (клієнтська область).
    pub source: Size,
    /// Куди малюється кадр (координати монітора).
    pub dest: Rect,
    /// Чи оверлей зараз видно (гра на передньому плані).
    pub visible: bool,
}

/// Спільний стан між потоком апскейлу й власником [`Upscaler`].
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub(crate) stop: AtomicBool,
    pub(crate) captured: AtomicU64,
    pub(crate) presented: AtomicU64,
    pub(crate) visible: AtomicBool,
    pub(crate) geometry: Mutex<(Size, Rect)>,
    pub(crate) end: Mutex<Option<EndReason>>,
}

impl Shared {
    pub(crate) fn set_geometry(&self, source: Size, dest: Rect) {
        if let Ok(mut g) = self.geometry.lock() {
            *g = (source, dest);
        }
    }

    pub(crate) fn finish(&self, reason: EndReason) {
        if let Ok(mut e) = self.end.lock() {
            e.get_or_insert(reason);
        }
    }
}

/// Запущений апскейл одного вікна.
pub struct Upscaler {
    shared: Arc<Shared>,
    wake: Arc<imp::Event>,
    thread: Option<JoinHandle<()>>,
}

impl Upscaler {
    /// Починає масштабувати вікно `target` на весь його монітор.
    /// Повертається, коли захоплення й оверлей готові (або з помилкою, чому ні).
    pub fn start(target: WindowId, settings: UpscaleSettings) -> Result<Self, UpscaleError> {
        let shared = Arc::new(Shared::default());
        let wake = Arc::new(imp::Event::new()?);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = {
            let shared = Arc::clone(&shared);
            let wake = Arc::clone(&wake);
            std::thread::Builder::new()
                .name("optimax-upscale".into())
                .spawn(move || imp::run(target, settings, &shared, &wake, ready_tx))
                .map_err(|e| UpscaleError::Thread(e.to_string()))?
        };
        let mut upscaler = Self {
            shared,
            wake,
            thread: Some(thread),
        };
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(upscaler),
            Ok(Err(e)) => {
                upscaler.join();
                Err(e)
            }
            Err(_) => {
                upscaler.join();
                Err(UpscaleError::Thread("не відповів під час запуску".into()))
            }
        }
    }

    /// Поточні лічильники.
    pub fn stats(&self) -> UpscaleStats {
        let (source, dest) = self.shared.geometry.lock().map(|g| *g).unwrap_or_default();
        UpscaleStats {
            captured: self.shared.captured.load(Ordering::Relaxed),
            presented: self.shared.presented.load(Ordering::Relaxed),
            source,
            dest,
            visible: self.shared.visible.load(Ordering::Relaxed),
        }
    }

    /// Чому сеанс завершився (`None` — ще працює).
    pub fn end_reason(&self) -> Option<EndReason> {
        self.shared.end.lock().ok().and_then(|e| e.clone())
    }

    /// Чи ще працює.
    pub fn is_running(&self) -> bool {
        self.end_reason().is_none()
    }

    /// Зупиняє апскейл і чекає завершення потоку. Повертає причину завершення.
    pub fn stop(mut self) -> EndReason {
        self.join();
        self.end_reason().unwrap_or(EndReason::Stopped)
    }

    fn join(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.wake.set();
        if let Some(t) = self.thread.take() {
            if t.join().is_err() {
                self.shared.finish(EndReason::Failed(
                    "потік апскейлу аварійно завершився".into(),
                ));
            }
        }
        self.shared.finish(EndReason::Stopped);
    }
}

impl Drop for Upscaler {
    fn drop(&mut self) {
        self.join();
    }
}
