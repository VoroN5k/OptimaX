//! Сеанс апскейлу: окремий потік із власним вікном-оверлеєм, захопленням і рендером.
//! Зупиняється викликом [`Upscaler::stop`], знищенням або сам — коли вікно гри закрили.

use crate::error::UpscaleError;
use crate::geometry::{Rect, Size};
use crate::imp;
use crate::measure::Summary;
use crate::settings::{Filter, UpscaleSettings};
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
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpscaleStats {
    /// Кадрів отримано від захоплення.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub captured: u64,
    /// Кадрів показано в оверлеї.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub presented: u64,
    /// Кадрів гри пропущено, бо вже не встигали на свій V-Sync (щоб не відставати на кадр).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub skipped: u64,
    /// Розмір кадру гри (клієнтська область).
    pub source: Size,
    /// Куди малюється кадр (координати монітора).
    pub dest: Rect,
    /// Фільтр, що фактично застосовано (NIS поза ×1…×2 замінюється білінійним).
    pub filter: Filter,
    /// Фільтр зробив лише частину масштабу (NIS понад ×2), решту — білінійний прохід.
    pub partial: bool,
    /// Чи оверлей зараз видно (гра на передньому плані).
    pub visible: bool,
    /// Виміри за останні ~5 с (див. [`Metrics`]).
    pub metrics: Metrics,
}

/// Виміряна вартість апскейлу (останні ~1000 кадрів).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Metrics {
    /// GPU-час масштабування одного кадру, мс (мітки часу D3D11).
    pub gpu_ms: Option<Summary>,
    /// Обробка в OptimaX: від отримання кадру гри до нашого Present, мс.
    pub latency_ms: Option<Summary>,
    /// Додана затримка, мс: наш кадр на екрані (статистика DXGI) мінус момент, на який
    /// Windows запланував показ кадру гри без апскейлу. `None` — статистики немає.
    pub display_ms: Option<Summary>,
    /// Частка кадрів, показаних хоч на один V-Sync пізніше, ніж без апскейлу, %.
    pub late_percent: Option<f64>,
    /// Завантаження потоку апскейлу за весь сеанс, % одного ядра CPU.
    pub cpu_percent: f64,
}

/// Поточна розкладка: розміри й фактичний фільтр.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Layout {
    pub(crate) source: Size,
    pub(crate) dest: Rect,
    pub(crate) filter: Filter,
    pub(crate) partial: bool,
}

/// Спільний стан між потоком апскейлу й власником [`Upscaler`].
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub(crate) stop: AtomicBool,
    pub(crate) captured: AtomicU64,
    pub(crate) presented: AtomicU64,
    pub(crate) skipped: AtomicU64,
    pub(crate) visible: AtomicBool,
    pub(crate) layout: Mutex<Layout>,
    pub(crate) metrics: Mutex<Metrics>,
    pub(crate) end: Mutex<Option<EndReason>>,
}

impl Shared {
    pub(crate) fn set_layout(&self, layout: Layout) {
        if let Ok(mut l) = self.layout.lock() {
            *l = layout;
        }
    }

    pub(crate) fn set_metrics(&self, metrics: Metrics) {
        if let Ok(mut m) = self.metrics.lock() {
            *m = metrics;
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
        let layout = self.shared.layout.lock().map(|l| *l).unwrap_or_default();
        UpscaleStats {
            captured: self.shared.captured.load(Ordering::Relaxed),
            presented: self.shared.presented.load(Ordering::Relaxed),
            skipped: self.shared.skipped.load(Ordering::Relaxed),
            source: layout.source,
            dest: layout.dest,
            filter: layout.filter,
            partial: layout.partial,
            visible: self.shared.visible.load(Ordering::Relaxed),
            metrics: self.shared.metrics.lock().map(|m| *m).unwrap_or_default(),
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
