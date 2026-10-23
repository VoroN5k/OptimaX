//! Direct3D 11: пристрій, компіляція HLSL системним `d3dcompiler_47.dll`, текстури, буфери
//! констант і два види проходів — піксельний (у задній буфер) і обчислювальний (у текстуру).

use super::os;
use crate::error::UpscaleError;
use crate::geometry::{Rect, Size};
use windows::core::{Interface, PCSTR};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_DRIVER_TYPE_HARDWARE,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Buffer, ID3D11ComputeShader, ID3D11Device, ID3D11DeviceContext,
    ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11UnorderedAccessView, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_UNORDERED_ACCESS, D3D11_BOX, D3D11_BUFFER_DESC,
    D3D11_COMPARISON_NEVER, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FILTER,
    D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_FLOAT32_MAX,
    D3D11_SAMPLER_DESC, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT, D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R32G32B32A32_FLOAT,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

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

/// Порожній результат `Create*` (не мав би траплятися при успішному HRESULT).
fn missing(api: &'static str) -> UpscaleError {
    UpscaleError::Os { api, code: 0 }
}

/// Компілює HLSL у байткод. `source` має бути без `#include` (див. `shaders::expand_includes`).
pub(super) fn compile(source: &str, entry: PCSTR, target: PCSTR) -> Result<Vec<u8>, UpscaleError> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: джерело — наш &str (вказівник і довжина узгоджені), рядки — статичні PCSTR,
    // результати пишуться в локальні Option.
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
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

pub(super) fn vertex_shader(
    gpu: &Gpu,
    source: &str,
    entry: PCSTR,
) -> Result<ID3D11VertexShader, UpscaleError> {
    let code = compile(source, entry, windows::core::s!("vs_5_0"))?;
    let mut shader = None;
    // SAFETY: байткод щойно скомпільований для цієї стадії.
    unsafe {
        gpu.device
            .CreateVertexShader(&code, None, Some(&mut shader))
    }
    .map_err(os("CreateVertexShader"))?;
    shader.ok_or_else(|| missing("CreateVertexShader"))
}

pub(super) fn pixel_shader(
    gpu: &Gpu,
    source: &str,
    entry: PCSTR,
) -> Result<ID3D11PixelShader, UpscaleError> {
    let code = compile(source, entry, windows::core::s!("ps_5_0"))?;
    let mut shader = None;
    // SAFETY: байткод щойно скомпільований для цієї стадії.
    unsafe { gpu.device.CreatePixelShader(&code, None, Some(&mut shader)) }
        .map_err(os("CreatePixelShader"))?;
    shader.ok_or_else(|| missing("CreatePixelShader"))
}

pub(super) fn compute_shader(gpu: &Gpu, source: &str) -> Result<ID3D11ComputeShader, UpscaleError> {
    let code = compile(
        source,
        windows::core::s!("main"),
        windows::core::s!("cs_5_0"),
    )?;
    let mut shader = None;
    // SAFETY: байткод щойно скомпільований для цієї стадії.
    unsafe {
        gpu.device
            .CreateComputeShader(&code, None, Some(&mut shader))
    }
    .map_err(os("CreateComputeShader"))?;
    shader.ok_or_else(|| missing("CreateComputeShader"))
}

pub(super) fn sampler(gpu: &Gpu, linear: bool) -> Result<ID3D11SamplerState, UpscaleError> {
    let filter: D3D11_FILTER = if linear {
        D3D11_FILTER_MIN_MAG_MIP_LINEAR
    } else {
        D3D11_FILTER_MIN_MAG_MIP_POINT
    };
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
    state.ok_or_else(|| missing("CreateSamplerState"))
}

/// Буфер констант із 32-бітних слів (розмір кратний 16 байтам).
pub(super) struct Constants {
    buffer: ID3D11Buffer,
    words: usize,
}

impl Constants {
    pub fn new(gpu: &Gpu, words: usize) -> Result<Self, UpscaleError> {
        let words = words.div_ceil(4) * 4;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: u32::try_from(words * 4).unwrap_or(u32::MAX),
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut buffer = None;
        // SAFETY: опис локальний, без початкових даних.
        unsafe { gpu.device.CreateBuffer(&desc, None, Some(&mut buffer)) }
            .map_err(os("CreateBuffer"))?;
        Ok(Self {
            buffer: buffer.ok_or_else(|| missing("CreateBuffer"))?,
            words,
        })
    }

    /// Записує значення (решта буфера — нулі).
    pub fn update(&self, gpu: &Gpu, values: &[u32]) {
        let mut data = vec![0u32; self.words];
        let n = values.len().min(self.words);
        data[..n].copy_from_slice(&values[..n]);
        // SAFETY: `data` має рівно розмір буфера й живе до кінця виклику.
        unsafe {
            gpu.context
                .UpdateSubresource(&self.buffer, 0, None, data.as_ptr().cast(), 0, 0);
        }
    }

    pub fn buffer(&self) -> &ID3D11Buffer {
        &self.buffer
    }
}

fn texture(
    gpu: &Gpu,
    size: Size,
    format: DXGI_FORMAT,
    bind: u32,
    init: Option<(&[u8], u32)>,
) -> Result<ID3D11Texture2D, UpscaleError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: if init.is_some() {
            D3D11_USAGE_IMMUTABLE
        } else {
            D3D11_USAGE_DEFAULT
        },
        BindFlags: bind,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let data = init.map(|(values, pitch)| D3D11_SUBRESOURCE_DATA {
        pSysMem: values.as_ptr().cast(),
        SysMemPitch: pitch,
        SysMemSlicePitch: 0,
    });
    let mut tex = None;
    // SAFETY: опис і дані локальні; дані (якщо є) покривають усю текстуру (див. викликачів).
    unsafe {
        gpu.device.CreateTexture2D(
            &desc,
            data.as_ref().map(|d| d as *const D3D11_SUBRESOURCE_DATA),
            Some(&mut tex),
        )
    }
    .map_err(os("CreateTexture2D"))?;
    tex.ok_or_else(|| missing("CreateTexture2D"))
}

fn srv(gpu: &Gpu, tex: &ID3D11Texture2D) -> Result<ID3D11ShaderResourceView, UpscaleError> {
    let mut view = None;
    // SAFETY: текстура створена з BIND_SHADER_RESOURCE.
    unsafe {
        gpu.device
            .CreateShaderResourceView(tex, None, Some(&mut view))
    }
    .map_err(os("CreateShaderResourceView"))?;
    view.ok_or_else(|| missing("CreateShaderResourceView"))
}

/// Кадр гри (клієнтська область), скопійований з кадру захоплення.
pub(super) struct SourceTexture {
    pub texture: ID3D11Texture2D,
    pub view: ID3D11ShaderResourceView,
    pub size: Size,
}

impl SourceTexture {
    pub fn new(gpu: &Gpu, size: Size) -> Result<Self, UpscaleError> {
        let texture = texture(
            gpu,
            size,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            D3D11_BIND_SHADER_RESOURCE.0 as u32,
            None,
        )?;
        let view = srv(gpu, &texture)?;
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

/// Проміжна текстура між проходами: пишеться обчислювальним шейдером, читається наступним.
/// RGBA8 — формат, який гарантовано підтримує запис з UAV на рівні 11.0.
pub(super) struct Intermediate {
    pub view: ID3D11ShaderResourceView,
    pub uav: ID3D11UnorderedAccessView,
    pub size: Size,
}

impl Intermediate {
    pub fn new(gpu: &Gpu, size: Size) -> Result<Self, UpscaleError> {
        let tex = texture(
            gpu,
            size,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_UNORDERED_ACCESS.0) as u32,
            None,
        )?;
        let view = srv(gpu, &tex)?;
        let mut uav = None;
        // SAFETY: текстура створена з BIND_UNORDERED_ACCESS.
        unsafe {
            gpu.device
                .CreateUnorderedAccessView(&tex, None, Some(&mut uav))
        }
        .map_err(os("CreateUnorderedAccessView"))?;
        Ok(Self {
            view,
            uav: uav.ok_or_else(|| missing("CreateUnorderedAccessView"))?,
            size,
        })
    }
}

/// Незмінна текстура float4 з таблицею (коефіцієнти NIS). `values.len()` = ширина × висота × 4.
pub(super) fn table_texture(
    gpu: &Gpu,
    size: Size,
    values: &[f32],
) -> Result<ID3D11ShaderResourceView, UpscaleError> {
    let expected = size.width as usize * size.height as usize * 4;
    if values.len() != expected {
        return Err(UpscaleError::Shader(format!(
            "таблиця має {} значень замість {expected}",
            values.len()
        )));
    }
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let tex = texture(
        gpu,
        size,
        DXGI_FORMAT_R32G32B32A32_FLOAT,
        D3D11_BIND_SHADER_RESOURCE.0 as u32,
        Some((&bytes, size.width * 16)),
    )?;
    srv(gpu, &tex)
}

/// Незмінна текстура BGRA8 з готових пікселів (`pixels.len()` = ширина × висота × 4).
pub(super) fn image_texture(
    gpu: &Gpu,
    size: Size,
    pixels: &[u8],
) -> Result<ID3D11ShaderResourceView, UpscaleError> {
    if pixels.len() != size.width as usize * size.height as usize * 4 {
        return Err(UpscaleError::Shader(
            "розмір зображення не збігається".into(),
        ));
    }
    let tex = texture(
        gpu,
        size,
        DXGI_FORMAT_B8G8R8A8_UNORM,
        D3D11_BIND_SHADER_RESOURCE.0 as u32,
        Some((pixels, size.width * 4)),
    )?;
    srv(gpu, &tex)
}

/// Піксельний прохід: повноекранний трикутник у межах `dest`.
pub(super) struct Draw<'a> {
    pub vs: &'a ID3D11VertexShader,
    pub ps: &'a ID3D11PixelShader,
    pub source: &'a ID3D11ShaderResourceView,
    pub sampler: Option<&'a ID3D11SamplerState>,
    pub constants: Option<&'a Constants>,
}

impl Draw<'_> {
    /// Очищує ціль чорним і малює.
    pub fn run(&self, gpu: &Gpu, target: &ID3D11RenderTargetView, dest: Rect) {
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
            ctx.VSSetShader(self.vs, None);
            ctx.PSSetShader(self.ps, None);
            ctx.PSSetShaderResources(0, Some(&[Some(self.source.clone())]));
            ctx.PSSetSamplers(0, Some(&[self.sampler.cloned()]));
            ctx.PSSetConstantBuffers(0, Some(&[self.constants.map(|c| c.buffer().clone())]));
            ctx.Draw(3, 0);
            // Відв'язуємо вхід, щоб наступний запис у цю текстуру не конфліктував з читанням.
            ctx.PSSetShaderResources(0, Some(&[None]));
        }
    }
}

/// Обчислювальний прохід у проміжну текстуру.
pub(super) struct Dispatch<'a> {
    pub cs: &'a ID3D11ComputeShader,
    pub sources: &'a [Option<ID3D11ShaderResourceView>],
    pub sampler: Option<&'a ID3D11SamplerState>,
    pub constants: &'a Constants,
    pub output: &'a Intermediate,
    pub groups: (u32, u32),
}

impl Dispatch<'_> {
    pub fn run(&self, gpu: &Gpu) {
        let ctx = &gpu.context;
        let uav = [Some(self.output.uav.clone())];
        let none_uav: [Option<ID3D11UnorderedAccessView>; 1] = [None];
        let none_srv = vec![None; self.sources.len()];
        // SAFETY: усі об'єкти створені на цьому пристрої; масиви живуть до кінця блоку.
        unsafe {
            ctx.CSSetShader(self.cs, None);
            ctx.CSSetShaderResources(0, Some(self.sources));
            ctx.CSSetSamplers(0, Some(&[self.sampler.cloned()]));
            ctx.CSSetConstantBuffers(0, Some(&[Some(self.constants.buffer().clone())]));
            ctx.CSSetUnorderedAccessViews(0, 1, Some(uav.as_ptr()), None);
            ctx.Dispatch(self.groups.0, self.groups.1, 1);
            // Відв'язуємо вихід і входи: далі текстура читається піксельним проходом.
            ctx.CSSetUnorderedAccessViews(0, 1, Some(none_uav.as_ptr()), None);
            ctx.CSSetShaderResources(0, Some(&none_srv));
        }
    }
}
