//! Захоплення вікна гри через Windows Graphics Capture.

use super::d3d::Gpu;
use super::{os, set_raw};
use crate::error::UpscaleError;
use crate::geometry::Size;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::{IInspectable, Interface};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

/// Буферів у пулі: один у нас, один пише система — мінімальна затримка.
const BUFFERS: i32 = 2;
const FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

/// Активне захоплення одного вікна.
pub(super) struct Capture {
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    pool_size: SizeInt32,
    arrived_token: i64,
    closed_token: i64,
    closed: Arc<AtomicBool>,
}

/// Отриманий кадр: текстура кадру й розмір вмісту.
pub(super) struct Frame {
    _frame: Direct3D11CaptureFrame,
    pub texture: ID3D11Texture2D,
    pub content: Size,
    /// Коли ми отримали кадр (тики QPC).
    pub received_qpc: i64,
    /// `SystemRelativeTime` кадру (QPC в одиницях по 100 нс). Заміри показують, що це момент
    /// V-Sync, на який Windows запланував показ кадру гри (лежить на сітці кадрів монітора й
    /// зазвичай трохи попереду моменту отримання) — тобто коли кадр з'явився б без апскейлу.
    pub scheduled_hns: i64,
}

impl Capture {
    /// Починає захоплення `hwnd`. Новий кадр і закриття вікна сигналять подію `wake`.
    pub fn start(gpu: &Gpu, hwnd: HWND, cursor: bool, wake: isize) -> Result<Self, UpscaleError> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(os("IGraphicsCaptureItemInterop"))?;
        // SAFETY: `hwnd` — вікно верхнього рівня, перевірене викликачем.
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForWindow(hwnd) }.map_err(os("CreateForWindow"))?;
        let pool_size = item.Size().map_err(os("GraphicsCaptureItem.Size"))?;
        let pool =
            Direct3D11CaptureFramePool::CreateFreeThreaded(&gpu.winrt, FORMAT, BUFFERS, pool_size)
                .map_err(os("CreateFreeThreaded"))?;

        let arrived_token = pool
            .FrameArrived(
                &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                    set_raw(wake);
                    Ok(())
                }),
            )
            .map_err(os("FrameArrived"))?;
        let closed = Arc::new(AtomicBool::new(false));
        let closed_token = {
            let closed = Arc::clone(&closed);
            item.Closed(
                &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                    closed.store(true, Ordering::Relaxed);
                    set_raw(wake);
                    Ok(())
                }),
            )
            .map_err(os("GraphicsCaptureItem.Closed"))?
        };

        let session = pool
            .CreateCaptureSession(&item)
            .map_err(os("CreateCaptureSession"))?;
        // Необов'язкові властивості: на старших збірках Windows їх немає — тоді лишаємо як є.
        let _ = session.SetIsCursorCaptureEnabled(cursor);
        // Жовта рамка захоплення (вимикається на Windows 11; на Windows 10 може лишитися).
        let _ = session.SetIsBorderRequired(false);
        // Без обмеження частоти кадрів захоплення (1 мс), щоб не різати високий FPS.
        let _ = session.SetMinUpdateInterval(TimeSpan { Duration: 10_000 });
        session.StartCapture().map_err(os("StartCapture"))?;

        Ok(Self {
            item,
            pool,
            session,
            pool_size,
            arrived_token,
            closed_token,
            closed,
        })
    }

    /// Чи закрили захоплюване вікно.
    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// Найсвіжіший кадр (старіші одразу повертаються в пул). `None` — нових кадрів немає.
    pub fn latest(&mut self, gpu: &Gpu) -> Result<Option<Frame>, UpscaleError> {
        let mut last = None;
        // Порожній пул повертає null, що для обгортки — помилка; це просто «кадрів немає».
        while let Ok(frame) = self.pool.TryGetNextFrame() {
            last = Some(frame);
        }
        let Some(frame) = last else {
            return Ok(None);
        };
        let content = frame.ContentSize().map_err(os("ContentSize"))?;
        if content.Width != self.pool_size.Width || content.Height != self.pool_size.Height {
            // Вікно змінило розмір: перебудовуємо пул під новий розмір (цей кадр ще валідний).
            self.pool
                .Recreate(&gpu.winrt, FORMAT, BUFFERS, content)
                .map_err(os("Recreate"))?;
            self.pool_size = content;
        }
        let access: IDirect3DDxgiInterfaceAccess = frame
            .Surface()
            .and_then(|s| s.cast())
            .map_err(os("Direct3D11CaptureFrame.Surface"))?;
        // SAFETY: поверхня кадру WGC — це текстура D3D11 нашого пристрою.
        let texture: ID3D11Texture2D =
            unsafe { access.GetInterface() }.map_err(os("IDirect3DDxgiInterfaceAccess"))?;
        let scheduled_hns = frame.SystemRelativeTime().map(|t| t.Duration).unwrap_or(0);
        Ok(Some(Frame {
            received_qpc: optimax_core::clock::qpc_now(),
            scheduled_hns,
            _frame: frame,
            texture,
            content: Size::new(
                u32::try_from(content.Width).unwrap_or(0),
                u32::try_from(content.Height).unwrap_or(0),
            ),
        }))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // Спершу знімаємо обробники — після цього вони не торкнуться дескриптора події.
        let _ = self.pool.RemoveFrameArrived(self.arrived_token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
