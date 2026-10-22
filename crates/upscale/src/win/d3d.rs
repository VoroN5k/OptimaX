//! Direct3D 11: пристрій, компіляція HLSL системним `d3dcompiler_47.dll`, прохід масштабування.

use super::os;
use crate::error::UpscaleError;
use crate::geometry::{Rect, Size};
use crate::settings::Filter;
use windows::core::{s, Interface, PCSTR};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_DRIVER_TYPE_HARDWARE,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D,
    ID3D11VertexShader, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_COMPARISON_NEVER,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FILTER, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_FLOAT32_MAX, D3D11_SAMPLER_DESC, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

const BLIT_HLSL: &str = include_str!("../shaders/blit.hlsl");

/// Пристрій D3D11 і його WinRT-обгортка (для пулу кадрів WGC).
pub(super) struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub winrt: IDirect3DDevice,
}

impl Gpu {
    /// Апаратний пристрій на адаптері за замовчуванням, рівень функцій 11.0+.
    pub fn new() -> Result<Self, UpscaleError> {
        let mut device = None;
        let mut context = None;
        // SAFETY: усі вказівники — на локальні Option, які заповнює функція.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(os("D3D11CreateDevice"))?;
        let (Some(device), Some(context)) = (device, context) else {
            return Err(UpscaleError::Unsupported("Direct3D 11.0"));
        };
        let dxgi: IDXGIDevice = device.cast().map_err(os("IDXGIDevice"))?;
        // SAFETY: `dxgi` — дійсний DXGI-пристрій щойно створеного D3D11-пристрою.
        let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .and_then(|i| i.cast())
            .map_err(os("CreateDirect3D11DeviceFromDXGIDevice"))?;
        Ok(Self {
            device,
            context,
            winrt,
        })
    }
}

/// Компілює HLSL у байткод.
pub(super) fn compile(
    source: &str,
    name: PCSTR,
    entry: PCSTR,
    target: PCSTR,
) -> Result<Vec<u8>, UpscaleError> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: джерело — наш &str (вказівник і довжина узгоджені), рядки — статичні PCSTR,
    // результати пишуться в локальні Option.
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            name,
            None,
            None,
            entry,
            target,
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    match (result, code) {
        (Ok(()), Some(code)) => Ok(blob_bytes(&code).to_vec()),
        (r, _) => {
            let msg = errors
                .as_ref()
                .map(|e| String::from_utf8_lossy(blob_bytes(e)).trim().to_owned())
                .filter(|m| !m.is_empty())
                .or_else(|| r.err().map(|e| e.message()))
                .unwrap_or_default();
            Err(UpscaleError::Shader(msg))
        }
    }
}

fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    // SAFETY: буфер належить blob і живе, поки живе позичене посилання на нього.
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    }
}

/// Кадр гри (клієнтська область), скопійований з кадру захоплення.
pub(super) struct SourceTexture {
    pub texture: ID3D11Texture2D,
    pub view: ID3D11ShaderResourceView,
    pub size: Size,
}

impl SourceTexture {
    pub fn new(gpu: &Gpu, size: Size) -> Result<Self, UpscaleError> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.width,
            Height: size.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: опис локальний, без початкових даних.
        unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .map_err(os("CreateTexture2D"))?;
        let texture = texture.ok_or(UpscaleError::Os {
            api: "CreateTexture2D",
            code: 0,
        })?;
        let mut view = None;
        // SAFETY: текстура щойно створена з BIND_SHADER_RESOURCE.
        unsafe {
            gpu.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))
        }
        .map_err(os("CreateShaderResourceView"))?;
        let view = view.ok_or(UpscaleError::Os {
            api: "CreateShaderResourceView",
            code: 0,
        })?;
        Ok(Self {
            texture,
            view,
            size,
        })
    }

    /// Копіює прямокутник `crop` кадру захоплення (розмір має збігатися з `self.size`).
    pub fn copy_from(&self, gpu: &Gpu, frame: &ID3D11Texture2D, crop: Rect) {
        let left = u32::try_from(crop.x).unwrap_or(0);
        let top = u32::try_from(crop.y).unwrap_or(0);
        let region = D3D11_BOX {
            left,
            top,
            front: 0,
            right: left + crop.width,
            bottom: top + crop.height,
            back: 1,
        };
        // SAFETY: обидві текстури на цьому пристрої й одного формату; область у межах кадру
        // (див. `geometry::client_crop`) і збігається з розміром призначення.
        unsafe {
            gpu.context
                .CopySubresourceRegion(&self.texture, 0, 0, 0, 0, frame, 0, Some(&region));
        }
    }
}

/// Прохід «намалювати кадр у прямокутнику з фільтром».
pub(super) struct Blitter {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    point: ID3D11SamplerState,
    linear: ID3D11SamplerState,
}

impl Blitter {
    pub fn new(gpu: &Gpu) -> Result<Self, UpscaleError> {
        let vs_code = compile(BLIT_HLSL, s!("blit.hlsl"), s!("vs_main"), s!("vs_5_0"))?;
        let ps_code = compile(BLIT_HLSL, s!("blit.hlsl"), s!("ps_main"), s!("ps_5_0"))?;
        let mut vs = None;
        let mut ps = None;
        // SAFETY: байткод щойно скомпільований для цих стадій.
        unsafe {
            gpu.device
                .CreateVertexShader(&vs_code, None, Some(&mut vs))
                .map_err(os("CreateVertexShader"))?;
            gpu.device
                .CreatePixelShader(&ps_code, None, Some(&mut ps))
                .map_err(os("CreatePixelShader"))?;
        }
        let (Some(vs), Some(ps)) = (vs, ps) else {
            return Err(UpscaleError::Shader("порожній шейдер".into()));
        };
        Ok(Self {
            vs,
            ps,
            point: sampler(gpu, D3D11_FILTER_MIN_MAG_MIP_POINT)?,
            linear: sampler(gpu, D3D11_FILTER_MIN_MAG_MIP_LINEAR)?,
        })
    }

    /// Очищує ціль чорним і малює `source` у прямокутнику `dest`.
    pub fn draw(
        &self,
        gpu: &Gpu,
        source: &SourceTexture,
        target: &ID3D11RenderTargetView,
        dest: Rect,
        filter: Filter,
    ) {
        let sampler = match filter {
            Filter::Nearest => &self.point,
            Filter::Bilinear => &self.linear,
        };
        let viewport = D3D11_VIEWPORT {
            TopLeftX: dest.x as f32,
            TopLeftY: dest.y as f32,
            Width: dest.width as f32,
            Height: dest.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let ctx = &gpu.context;
        // SAFETY: усі об'єкти створені на цьому пристрої; масиви — локальні.
        unsafe {
            ctx.ClearRenderTargetView(target, &[0.0, 0.0, 0.0, 1.0]);
            ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            ctx.RSSetViewports(Some(&[viewport]));
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(None);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetShaderResources(0, Some(&[Some(source.view.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
            ctx.Draw(3, 0);
            // Відв'язуємо кадр, щоб наступне копіювання в нього не конфліктувало з вибіркою.
            ctx.PSSetShaderResources(0, Some(&[None]));
        }
    }
}

fn sampler(gpu: &Gpu, filter: D3D11_FILTER) -> Result<ID3D11SamplerState, UpscaleError> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: filter,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: D3D11_FLOAT32_MAX,
    };
    let mut state = None;
    // SAFETY: опис локальний.
    unsafe { gpu.device.CreateSamplerState(&desc, Some(&mut state)) }
        .map_err(os("CreateSamplerState"))?;
    state.ok_or(UpscaleError::Os {
        api: "CreateSamplerState",
        code: 0,
    })
}
