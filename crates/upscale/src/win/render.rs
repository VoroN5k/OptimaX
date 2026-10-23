//! Цикл потоку апскейлу: чекаємо кадр / стоп / повідомлення вікна, копіюємо клієнтську
//! область кадру, малюємо її збільшеною в оверлеї. Оверлей видно, лише поки гра на передньому плані.

use super::capture::Capture;
use super::cursor::CursorControl;
use super::cursor_layer::CursorLayer;
use super::cursor_shape::{self, CursorState};
use super::d3d::{Gpu, SourceTexture};
use super::overlay::Overlay;
use super::pipeline::Pipeline;
use super::timing::{thread_cpu_time, DisplayTracker, GpuTimer};
use super::window::{self, Geometry};
use super::Event;
use crate::cursor::cursor_rect;
use crate::error::UpscaleError;
use crate::geometry::{client_crop, dest_rect, needs_upscale, Rect};
use crate::measure::{
    cpu_percent, qpc_to_100ns, should_skip_frame, Samples, LATE_THRESHOLD_MS, WINDOW,
};
use crate::session::{EndReason, Layout, Metrics, Shared};
use crate::settings::UpscaleSettings;
use crate::window::WindowId;
use optimax_core::clock::{qpc_frequency, qpc_now, ticks_to_ms};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use windows::Graphics::Capture::GraphicsCaptureSession;
use windows::Win32::Foundation::{HWND, WAIT_EVENT, WAIT_OBJECT_0};
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    QS_ALLINPUT,
};

/// Як часто перевіряти вікно гри (позиція, фокус, монітор), навіть без нових кадрів.
const CHECK_EVERY: Duration = Duration::from_millis(100);
/// Як часто перевіряти курсор, поки оверлей видно: курсор має рухатися плавно, навіть коли
/// гра не малює нових кадрів (меню, пауза).
const CURSOR_EVERY: Duration = Duration::from_millis(4);
/// Гра «стоїть» (меню, пауза), якщо нового кадру не було довше за це. Лише тоді курсор
/// перемальовується окремо: коли гра малює, курсор оновлюється з кожним її кадром, а зайві
/// показ займали б чергу й збивали синхронізацію з V-Sync.
const GAME_IDLE: Duration = Duration::from_millis(20);
/// Як часто оновлювати зведення вимірів для [`Shared`].
const PUBLISH_EVERY: Duration = Duration::from_millis(500);

/// Виміри вартості апскейлу в потоці рендера.
struct Meter {
    gpu_timer: GpuTimer,
    display: DisplayTracker,
    frequency: i64,
    gpu_ms: Samples,
    latency_ms: Samples,
    display_ms: Samples,
    /// Процесорний час потоку й момент на старті сеансу (CPU рахуємо за весь сеанс:
    /// Windows обліковує час потоку квантами ~15,6 мс, короткі інтервали дають шум).
    cpu_start: (u64, Instant),
    published: Instant,
}

impl Meter {
    fn new(gpu: &Gpu) -> Result<Self, UpscaleError> {
        let frequency = qpc_frequency();
        Ok(Self {
            gpu_timer: GpuTimer::new(gpu)?,
            display: DisplayTracker::new(frequency),
            frequency,
            gpu_ms: Samples::new(WINDOW),
            latency_ms: Samples::new(WINDOW),
            display_ms: Samples::new(WINDOW),
            cpu_start: (thread_cpu_time(), Instant::now()),
            published: Instant::now(),
        })
    }

    /// Після Present: час обробки (отримали кадр → Present) і запис для пошуку моменту показу.
    /// `scheduled_hns` — коли Windows запланував показати цей кадр гри без апскейлу.
    fn presented(&mut self, overlay: &Overlay, received_qpc: i64, scheduled_hns: i64) {
        if received_qpc != 0 {
            self.latency_ms
                .push(ticks_to_ms(qpc_now() - received_qpc, self.frequency));
        }
        if scheduled_hns != 0 {
            self.display.presented(overlay.swap(), scheduled_hns);
        }
    }

    /// Забирає готові результати й час від часу публікує зведення.
    fn tick(&mut self, gpu: &Gpu, overlay: &Overlay, shared: &Shared) {
        let samples = &mut self.gpu_ms;
        self.gpu_timer.collect(gpu, |ms| samples.push(ms));
        if let Some(ms) = self.display.poll(overlay.swap()) {
            self.display_ms.push(ms);
        }
        if self.published.elapsed() < PUBLISH_EVERY {
            return;
        }
        self.published = Instant::now();
        let (cpu_then, wall_then) = self.cpu_start;
        let wall = u64::try_from(wall_then.elapsed().as_nanos() / 100).unwrap_or(0);
        shared.set_metrics(Metrics {
            gpu_ms: self.gpu_ms.summary(),
            latency_ms: self.latency_ms.summary(),
            display_ms: self.display_ms.summary(),
            late_percent: self.display_ms.share_above(LATE_THRESHOLD_MS),
            cpu_percent: cpu_percent(thread_cpu_time().saturating_sub(cpu_then), wall),
        });
    }
}

pub(crate) fn run(
    target: WindowId,
    settings: UpscaleSettings,
    shared: &Shared,
    wake: &Event,
    ready: Sender<Result<(), UpscaleError>>,
) {
    // SAFETY: ініціалізація WinRT для цього (нового) потоку; парний RoUninitialize нижче.
    let ro = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
    match Renderer::new(target, settings, wake) {
        Ok(mut renderer) => {
            let _ = ready.send(Ok(()));
            let reason = renderer.run(shared, wake);
            drop(renderer);
            shared.visible.store(false, Ordering::Relaxed);
            shared.finish(reason);
        }
        Err(e) => {
            shared.finish(EndReason::Failed(e.to_string()));
            let _ = ready.send(Err(e));
        }
    }
    if ro {
        // SAFETY: парний до успішного RoInitialize; усі об'єкти WinRT уже знищені.
        unsafe { RoUninitialize() };
    }
}

struct Renderer {
    hwnd: HWND,
    settings: UpscaleSettings,
    gpu: Gpu,
    pipeline: Pipeline,
    overlay: Overlay,
    capture: Capture,
    cursor: CursorControl,
    cursor_layer: CursorLayer,
    /// Курсор, намальований в останньому кадрі.
    drawn_cursor: Option<CursorState>,
    geometry: Geometry,
    source: Option<SourceTexture>,
    /// Кадр гри, що зараз у `source`: коли ми його отримали (QPC) і коли Windows
    /// запланував його показ без апскейлу (100 нс).
    source_time: (i64, i64),
    /// Кадр гри, для якого вже записано виміри.
    measured_time: (i64, i64),
    dest: Rect,
    layout: Layout,
    meter: Meter,
}

impl Renderer {
    fn new(
        target: WindowId,
        settings: UpscaleSettings,
        wake: &Event,
    ) -> Result<Self, UpscaleError> {
        let hwnd = window::hwnd(target);
        let info = super::window_info(target).ok_or(UpscaleError::NoWindow)?;
        if info.pid == std::process::id() {
            return Err(UpscaleError::OwnWindow);
        }
        let geometry = window::geometry(hwnd).ok_or(UpscaleError::NoWindow)?;
        if geometry.minimized {
            return Err(UpscaleError::Minimized);
        }
        if !needs_upscale(geometry.client.size(), geometry.monitor.size()) {
            return Err(UpscaleError::AlreadyFullSize {
                client: geometry.client.size(),
                output: geometry.monitor.size(),
            });
        }
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err(UpscaleError::Unsupported("Windows Graphics Capture"));
        }

        let gpu = Gpu::new()?;
        let pipeline = Pipeline::new(&gpu, &settings)?;
        let overlay = Overlay::new(&gpu, geometry.monitor)?;
        // Курсор у кадрі WGC вимкнено: системний курсор ми ховаємо, а схованого WGC не малює,
        // тож збільшений курсор малює `CursorLayer`.
        let capture = Capture::start(&gpu, hwnd, false, wake.raw())?;
        let cursor_layer = CursorLayer::new(&gpu)?;
        let meter = Meter::new(&gpu)?;
        Ok(Self {
            hwnd,
            settings,
            gpu,
            pipeline,
            overlay,
            capture,
            cursor: CursorControl::new(),
            cursor_layer,
            drawn_cursor: None,
            geometry,
            source: None,
            source_time: (0, 0),
            measured_time: (0, 0),
            dest: Rect::default(),
            layout: Layout::default(),
            meter,
        })
    }

    fn run(&mut self, shared: &Shared, wake: &Event) -> EndReason {
        let mut last_check: Option<Instant> = None;
        let mut dirty = false;
        // Місце в черзі показу вже «зайняте» нами (успішне очікування дескриптора swap chain).
        let mut reserved = false;
        // Є кадр для показу, але черга показу зайнята — чекаємо й на неї.
        let mut waiting = false;
        let mut last_fresh: Option<Instant> = None;
        let mut skipped_last = false;
        loop {
            let handles = [wake.handle(), self.overlay.waitable()];
            let count = if waiting { 2 } else { 1 };
            let timeout = if self.draws_cursor(shared) {
                CURSOR_EVERY
            } else {
                CHECK_EVERY
            };
            // SAFETY: масив дескрипторів локальний, обидва живуть довше за виклик.
            let woke = unsafe {
                MsgWaitForMultipleObjects(
                    Some(&handles[..count]),
                    false,
                    timeout.as_millis() as u32,
                    QS_ALLINPUT,
                )
            };
            if waiting && woke == WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
                reserved = true;
                waiting = false;
            }
            pump_messages();

            if shared.stop.load(Ordering::Relaxed) {
                return EndReason::Stopped;
            }
            if self.capture.closed() {
                return EndReason::WindowClosed;
            }

            if last_check.is_none_or(|t| t.elapsed() >= CHECK_EVERY) {
                last_check = Some(Instant::now());
                match self.check_window(shared) {
                    Ok(Some(redraw)) => dirty |= redraw,
                    Ok(None) => return EndReason::WindowClosed,
                    Err(e) => return EndReason::Failed(e.to_string()),
                }
            }

            match self.take_frame(shared) {
                Ok(true) => {
                    dirty = true;
                    last_fresh = Some(Instant::now());
                }
                Ok(false) => {}
                Err(e) => return EndReason::Failed(e.to_string()),
            }
            let game_idle = last_fresh.is_none_or(|t| t.elapsed() >= GAME_IDLE);
            if game_idle && self.draws_cursor(shared) && self.cursor_now() != self.drawn_cursor {
                dirty = true;
            }

            if dirty && self.overlay_visible(shared) {
                reserved = reserved || self.overlay.try_reserve();
                if reserved && self.misses_vsync(skipped_last) {
                    // Кадр уже не встигає на свій V-Sync: чекаємо наступний, місце лишаємо за собою.
                    skipped_last = true;
                    dirty = false;
                    shared.skipped.fetch_add(1, Ordering::Relaxed);
                } else if reserved {
                    skipped_last = false;
                    // Малюємо найсвіжіший кадр (старіші вже відкинуті в `take_frame`).
                    match self.draw(shared) {
                        // Місце в черзі звільнить лише показаний кадр; якщо показувати ще
                        // нічого (перший кадр гри не прийшов), місце лишається за нами.
                        Ok(presented) => reserved = !presented,
                        Err(e) => return EndReason::Failed(e.to_string()),
                    }
                    dirty = false;
                } else {
                    waiting = true;
                }
            }
            self.meter.tick(&self.gpu, &self.overlay, shared);
        }
    }

    /// Оновлює геометрію й видимість. `None` — вікна гри більше немає;
    /// `Some(true)` — треба перемалювати (оверлей щойно з'явився або змінився монітор).
    fn check_window(&mut self, shared: &Shared) -> Result<Option<bool>, UpscaleError> {
        let Some(geometry) = window::geometry(self.hwnd) else {
            return Ok(None);
        };
        let mut redraw = false;
        if geometry.monitor != self.overlay.rect() {
            self.overlay.set_rect(&self.gpu, geometry.monitor)?;
            redraw = true;
        }
        self.geometry = geometry;
        let focused = window::foreground_root() == Some(self.hwnd);
        let visible = focused && !geometry.minimized;
        if self.overlay.set_visible(visible) {
            shared.visible.store(visible, Ordering::Relaxed);
            redraw |= visible;
        }
        // Повторюємо щоразу: Windows скидає обмеження курсора при деяких подіях (UAC, Ctrl+Alt+Del).
        self.cursor
            .update(visible, geometry.client, self.settings.capture_cursor);
        Ok(Some(redraw))
    }

    fn overlay_visible(&self, shared: &Shared) -> bool {
        shared.visible.load(Ordering::Relaxed)
    }

    /// Чи новий кадр гри вже не встигає на свій V-Sync (див. [`should_skip_frame`]).
    fn misses_vsync(&self, skipped_last: bool) -> bool {
        let fresh = self.source_time != self.measured_time;
        let now = qpc_to_100ns(qpc_now(), self.meter.frequency);
        fresh && should_skip_frame(now, self.source_time.1, skipped_last)
    }

    /// Чи малюємо власний курсор зараз (оверлей видно й збільшений курсор увімкнено).
    fn draws_cursor(&self, shared: &Shared) -> bool {
        self.settings.capture_cursor && self.overlay_visible(shared)
    }

    /// Видимий курсор над вікном гри (`None` — поза грою або гра його сховала).
    fn cursor_now(&self) -> Option<CursorState> {
        cursor_shape::current().filter(|c| {
            let g = self.geometry.client;
            c.pos.0 >= g.x
                && c.pos.1 >= g.y
                && c.pos.0 < g.x + g.width as i32
                && c.pos.1 < g.y + g.height as i32
        })
    }

    /// Забирає найсвіжіший кадр і копіює з нього клієнтську область. `true` — є новий кадр.
    fn take_frame(&mut self, shared: &Shared) -> Result<bool, UpscaleError> {
        let Some(frame) = self.capture.latest(&self.gpu)? else {
            return Ok(false);
        };
        shared.captured.fetch_add(1, Ordering::Relaxed);
        let Some(crop) = client_crop(self.geometry.frame, frame.content, self.geometry.client)
        else {
            return Ok(false);
        };
        if self.source.as_ref().map(|s| s.size) != Some(crop.size()) {
            self.source = Some(SourceTexture::new(&self.gpu, crop.size())?);
        }
        self.dest = dest_rect(crop.size(), self.overlay.rect().size(), self.settings.fit);
        if let Some(source) = &self.source {
            source.copy_from(&self.gpu, &frame.texture, crop);
            self.source_time = (frame.received_qpc, frame.scheduled_hns);
        }
        Ok(true)
    }

    /// Малює й показує кадр. `false` — показувати ще нічого.
    fn draw(&mut self, shared: &Shared) -> Result<bool, UpscaleError> {
        let (Some(source), Some(target)) = (&self.source, self.overlay.target()) else {
            return Ok(false);
        };
        self.pipeline.prepare(&self.gpu, source.size, self.dest)?;
        let timed = self.meter.gpu_timer.begin(&self.gpu);
        self.pipeline.render(&self.gpu, source, target, self.dest);
        self.drawn_cursor = None;
        if self.settings.capture_cursor {
            if let Some(state) = self.cursor_now() {
                if let Some((size, hotspot)) = self.cursor_layer.prepare(&self.gpu, state) {
                    let rect = cursor_rect(
                        state.pos,
                        hotspot,
                        size,
                        self.geometry.client,
                        source.size,
                        self.dest,
                    );
                    if let Some(rect) = rect {
                        self.cursor_layer.draw(&self.gpu, target, rect);
                    }
                }
                self.drawn_cursor = Some(state);
            }
        }
        if timed {
            self.meter.gpu_timer.end(&self.gpu);
        }
        self.overlay.present()?;
        // Міряємо лише показ нового кадру гри: перемальовування того самого кадру через рух
        // курсора — не затримка картинки.
        if self.source_time != self.measured_time {
            let (received, scheduled) = self.source_time;
            self.meter.presented(&self.overlay, received, scheduled);
            self.measured_time = self.source_time;
        }
        let applied = self.pipeline.applied();
        let layout = Layout {
            source: source.size,
            dest: self.dest,
            filter: applied.filter,
            partial: applied.partial,
        };
        if layout != self.layout {
            self.layout = layout;
            shared.set_layout(layout);
        }
        shared.presented.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    }
}

/// Обробляє повідомлення вікна-оверлею (без цього Windows вважатиме потік «завислим»).
fn pump_messages() {
    let mut msg = MSG::default();
    // SAFETY: `msg` — локальна структура; повідомлення лише цього потоку.
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
