//! Цикл потоку апскейлу: чекаємо кадр / стоп / повідомлення вікна, копіюємо клієнтську
//! область кадру, малюємо її збільшеною в оверлеї. Оверлей видно, лише поки гра на передньому плані.

use super::capture::Capture;
use super::cursor::CursorControl;
use super::d3d::{Gpu, SourceTexture};
use super::overlay::Overlay;
use super::pipeline::Pipeline;
use super::window::{self, Geometry};
use super::Event;
use crate::error::UpscaleError;
use crate::geometry::{client_crop, dest_rect, needs_upscale, Rect};
use crate::session::{EndReason, Layout, Shared};
use crate::settings::UpscaleSettings;
use crate::window::WindowId;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use windows::Graphics::Capture::GraphicsCaptureSession;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    QS_ALLINPUT,
};

/// Як часто перевіряти вікно гри (позиція, фокус, монітор), навіть без нових кадрів.
const CHECK_EVERY: Duration = Duration::from_millis(100);

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
    geometry: Geometry,
    source: Option<SourceTexture>,
    dest: Rect,
    layout: Layout,
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
        let capture = Capture::start(&gpu, hwnd, settings.capture_cursor, wake.raw())?;
        Ok(Self {
            hwnd,
            settings,
            gpu,
            pipeline,
            overlay,
            capture,
            cursor: CursorControl::new(),
            geometry,
            source: None,
            dest: Rect::default(),
            layout: Layout::default(),
        })
    }

    fn run(&mut self, shared: &Shared, wake: &Event) -> EndReason {
        let handles = [wake.handle()];
        let mut last_check: Option<Instant> = None;
        let mut dirty = false;
        loop {
            // SAFETY: масив дескрипторів локальний, подія живе довше за цикл.
            let _ = unsafe {
                MsgWaitForMultipleObjects(
                    Some(&handles),
                    false,
                    CHECK_EVERY.as_millis() as u32,
                    QS_ALLINPUT,
                )
            };
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
                Ok(fresh) => dirty |= fresh,
                Err(e) => return EndReason::Failed(e.to_string()),
            }

            if dirty && self.overlay_visible(shared) {
                if let Err(e) = self.draw(shared) {
                    return EndReason::Failed(e.to_string());
                }
                dirty = false;
            }
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
        }
        Ok(true)
    }

    fn draw(&mut self, shared: &Shared) -> Result<(), UpscaleError> {
        let (Some(source), Some(target)) = (&self.source, self.overlay.target()) else {
            return Ok(());
        };
        self.pipeline.prepare(&self.gpu, source.size, self.dest)?;
        self.pipeline.render(&self.gpu, source, target, self.dest);
        self.overlay.present()?;
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
        Ok(())
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
