//! Вікно-оверлей поверх гри: на весь монітор, завжди зверху, не забирає фокус,
//! пропускає мишу (WS_EX_TRANSPARENT + WS_EX_LAYERED). Малюється через flip-model swap chain.

use super::d3d::Gpu;
use super::os;
use crate::error::UpscaleError;
use crate::geometry::Rect;
use windows::core::{w, Interface};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D11::{ID3D11RenderTargetView, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1, DXGI_MWA_NO_ALT_ENTER,
    DXGI_MWA_NO_WINDOW_CHANGES, DXGI_PRESENT, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SetLayeredWindowAttributes,
    SetWindowPos, ShowWindow, HTTRANSPARENT, HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WM_MOUSEACTIVATE,
    WM_NCHITTEST, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS: windows::core::PCWSTR = w!("OptimaXUpscaleOverlay");
const BUFFERS: u32 = 2;

pub(super) struct Overlay {
    hwnd: HWND,
    swap: IDXGISwapChain1,
    target: Option<ID3D11RenderTargetView>,
    rect: Rect,
    visible: bool,
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Миша «проходить крізь» вікно, клік не активує його.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: стандартна обробка решти повідомлень з тими самими аргументами.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl Overlay {
    /// Створює приховане вікно на прямокутнику `rect` (монітор) і swap chain для нього.
    pub fn new(gpu: &Gpu, rect: Rect) -> Result<Self, UpscaleError> {
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
        // SAFETY: структура локальна; повторна реєстрація того самого класу (другий сеанс)
        // повертає 0 — це не помилка, клас уже є.
        unsafe { RegisterClassExW(&class) };

        // SAFETY: клас зареєстровано вище; рядки статичні.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                CLASS,
                w!("OptimaX upscale"),
                WS_POPUP,
                rect.x,
                rect.y,
                i32::try_from(rect.width).unwrap_or(i32::MAX),
                i32::try_from(rect.height).unwrap_or(i32::MAX),
                None,
                None,
                Some(instance),
                None,
            )
        }
        .map_err(os("CreateWindowExW"))?;

        match Self::attach(gpu, hwnd, rect) {
            Ok(o) => Ok(o),
            Err(e) => {
                // SAFETY: вікно створене цим потоком і ще ніде не збережене.
                let _ = unsafe { DestroyWindow(hwnd) };
                Err(e)
            }
        }
    }

    fn attach(gpu: &Gpu, hwnd: HWND, rect: Rect) -> Result<Self, UpscaleError> {
        // Повністю непрозоре «шарувате» вікно: шар потрібен лише для WS_EX_TRANSPARENT.
        // SAFETY: `hwnd` щойно створене.
        unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) }
            .map_err(os("SetLayeredWindowAttributes"))?;

        let factory = factory(gpu)?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: rect.width,
            Height: rect.height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: BUFFERS,
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        // SAFETY: пристрій і вікно дійсні; опис локальний.
        let swap = unsafe { factory.CreateSwapChainForHwnd(&gpu.device, hwnd, &desc, None, None) }
            .map_err(os("CreateSwapChainForHwnd"))?;
        // Alt+Enter і зміни вікна DXGI не чіпає — вікном керуємо самі.
        // SAFETY: вікно дійсне.
        unsafe {
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER | DXGI_MWA_NO_WINDOW_CHANGES)
        }
        .map_err(os("MakeWindowAssociation"))?;

        let mut overlay = Self {
            hwnd,
            swap,
            target: None,
            rect,
            visible: false,
        };
        overlay.target = Some(overlay.create_target(gpu)?);
        Ok(overlay)
    }

    fn create_target(&self, gpu: &Gpu) -> Result<ID3D11RenderTargetView, UpscaleError> {
        // SAFETY: буфер 0 flip-model swap chain у D3D11 завжди вказує на поточний задній буфер.
        let buffer: ID3D11Texture2D =
            unsafe { self.swap.GetBuffer(0) }.map_err(os("IDXGISwapChain.GetBuffer"))?;
        let mut view = None;
        // SAFETY: буфер swap chain створено з RENDER_TARGET_OUTPUT.
        unsafe {
            gpu.device
                .CreateRenderTargetView(&buffer, None, Some(&mut view))
        }
        .map_err(os("CreateRenderTargetView"))?;
        view.ok_or(UpscaleError::Os {
            api: "CreateRenderTargetView",
            code: 0,
        })
    }

    /// Прямокутник вікна (монітор).
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Ціль рендера (задній буфер).
    pub fn target(&self) -> Option<&ID3D11RenderTargetView> {
        self.target.as_ref()
    }

    /// Переносить вікно на інший монітор / розмір і перебудовує буфери.
    pub fn set_rect(&mut self, gpu: &Gpu, rect: Rect) -> Result<(), UpscaleError> {
        if rect == self.rect {
            return Ok(());
        }
        // SAFETY: вікно наше; прапорці не активують його.
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                rect.x,
                rect.y,
                i32::try_from(rect.width).unwrap_or(i32::MAX),
                i32::try_from(rect.height).unwrap_or(i32::MAX),
                SWP_NOACTIVATE,
            )
        }
        .map_err(os("SetWindowPos"))?;
        // Перед ResizeBuffers не має лишатися жодних посилань на буфери.
        self.target = None;
        // SAFETY: контекст пристрою — того самого, що й swap chain.
        unsafe {
            gpu.context.OMSetRenderTargets(None, None);
            gpu.context.Flush();
            self.swap.ResizeBuffers(
                BUFFERS,
                rect.width,
                rect.height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        }
        .map_err(os("ResizeBuffers"))?;
        self.rect = rect;
        self.target = Some(self.create_target(gpu)?);
        Ok(())
    }

    /// Показує / ховає вікно. Повертає `true`, якщо стан змінився.
    pub fn set_visible(&mut self, visible: bool) -> bool {
        if visible == self.visible {
            return false;
        }
        // SAFETY: вікно наше; SW_SHOWNOACTIVATE не забирає фокус у гри.
        unsafe {
            let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
            if visible {
                // Над грою, навіть якщо вона сама «завжди зверху».
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
        self.visible = visible;
        true
    }

    /// Показує намальований кадр. Синхронізація 0: DWM бере найсвіжіший кадр на V-Sync,
    /// без очікування в нашому потоці.
    pub fn present(&self) -> Result<(), UpscaleError> {
        // SAFETY: swap chain дійсний.
        unsafe { self.swap.Present(0, DXGI_PRESENT(0)) }
            .ok()
            .map_err(os("IDXGISwapChain.Present"))
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.target = None;
        // SAFETY: вікно створене цим самим потоком (оверлей не покидає потік апскейлу).
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

fn factory(gpu: &Gpu) -> Result<IDXGIFactory2, UpscaleError> {
    let dxgi: IDXGIDevice = gpu.device.cast().map_err(os("IDXGIDevice"))?;
    // SAFETY: DXGI-пристрій дійсний; фабрика — та, що створила його адаптер.
    let adapter: IDXGIAdapter = unsafe { dxgi.GetAdapter() }.map_err(os("GetAdapter"))?;
    // SAFETY: як вище.
    unsafe { adapter.GetParent() }.map_err(os("IDXGIAdapter.GetParent"))
}
