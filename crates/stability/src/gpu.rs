//! Навантаження GPU: обчислювальні шейдери Direct3D 11.
//!
//! Одне завдання: шейдер заповнює буфер із числа завдання, двічі перетворює його (ціла арифметика,
//! дробова арифметика, випадкове читання відеопам'яті) і згортає результат у невеликий відбиток.
//! Шейдер не залежить від порядку виконання потоків, тож результат має бути однаковим до біта.
//!
//! Робота ділиться на короткі порції, що чекають на GPU: одна порція не наближається до межі TDR
//! (2 с), тож скидання драйвера під час тесту означає збій, а не надто довгу команду.

use crate::check::{Digest, DIGEST_WORDS};
use crate::worker::{Load, LoadError};
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Buffer, ID3D11ComputeShader, ID3D11Device, ID3D11DeviceContext,
    ID3D11Query, ID3D11ShaderResourceView, ID3D11UnorderedAccessView, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_UNORDERED_ACCESS, D3D11_BUFFER_DESC,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_QUERY_DESC, D3D11_QUERY_EVENT, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, D3D11_SDK_VERSION,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_ERROR_DRIVER_INTERNAL_ERROR,
};

const SHADER: &str = r#"
cbuffer Params : register(b0) {
    uint seed;
    uint offset;   // перший елемент порції; кратний 256
    uint count;    // елементів у буфері; кратний 256
    uint inner;    // ітерацій обчислень на елемент
};
StructuredBuffer<uint4> src : register(t0);
RWStructuredBuffer<uint4> dst : register(u0);
RWStructuredBuffer<uint> digest : register(u1);
groupshared uint group_hash[256];

uint mix(uint x) {
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    return x;
}

[numthreads(256, 1, 1)]
void fill(uint3 id : SV_DispatchThreadID) {
    uint i = id.x + offset;
    uint a = mix(i * 2654435761u + seed + 1u);
    uint b = mix(a + 0x9E3779B9u);
    uint c = mix(b + 0x9E3779B9u);
    dst[i] = uint4(a, b, c, mix(c + 0x9E3779B9u));
}

[numthreads(256, 1, 1)]
void transform(uint3 id : SV_DispatchThreadID, uint index : SV_GroupIndex, uint3 group : SV_GroupID) {
    uint i = id.x + offset;
    uint4 v = src[i];
    uint r = mix(v.x ^ seed ^ i);
    // Випадкове читання: помилка відеопам'яті чи її шини псує і цей елемент.
    uint4 far_value = src[r % count];
    float f = float(v.y & 0xFFFFFFu) / 16777216.0 + 0.5;
    [loop] for (uint k = 0; k < inner; k++) {
        // Частина ітерацій знову читає довільне місце буфера: навантаження й на пам'ять.
        if ((k & 31u) == 31u) {
            far_value ^= src[r % count];
        }
        r = mix(r + k + far_value.z);
        f = frac(sqrt(f * f + 1.0) * 3.14159274 + sin(f * 6.28318548) * 0.5 + float(r & 0xFFFFu) / 65536.0);
    }
    uint4 o = uint4(r ^ far_value.x, asuint(f) ^ v.y, v.z + far_value.y, v.w ^ r);
    dst[i] = o;
    group_hash[index] = (o.x * 16777619u) ^ o.y ^ (o.z * 2166136261u) ^ o.w;
    GroupMemoryBarrierWithGroupSync();
    if (index == 0) {
        uint h = 2166136261u;
        for (uint j = 0; j < 256; j++) {
            h = (h ^ group_hash[j]) * 16777619u;
        }
        uint absolute = offset / 256 + group.x;
        InterlockedAdd(digest[absolute % 64], h);
    }
}
"#;

/// Потоків у групі шейдера.
const GROUP_THREADS: u32 = 256;
/// Розмір елемента буфера, байт.
const ELEMENT_BYTES: u32 = 16;
/// Найбільше очікування однієї порції. TDR спрацьовує значно раніше, тож це страховка.
const WAIT_LIMIT: Duration = Duration::from_secs(30);
/// Найменша порція, груп (стартова теж: перша порція має бути короткою на будь-якому GPU).
const MIN_GROUPS: u32 = 64;
/// Найбільша порція, груп: більшу команду Direct3D 11 мовчки відкидає (межа API), і без цього
/// обмеження тест звіряв би порожні результати. Дорівнює
/// `D3D11_CS_DISPATCH_MAX_THREAD_GROUPS_PER_DIMENSION`.
const MAX_GROUPS: u32 = 65_535;

/// Параметри навантаження.
#[derive(Debug, Clone)]
pub struct GpuParams {
    /// Найбільший розмір одного з двох буферів, МБ.
    pub buffer_mb: u64,
    /// Ітерацій обчислень на елемент.
    pub inner: u32,
    /// Бажана тривалість порції.
    pub slice: Duration,
}

impl Default for GpuParams {
    fn default() -> Self {
        Self {
            buffer_mb: 1024,
            inner: 64,
            slice: Duration::from_millis(100),
        }
    }
}

fn is_device_lost(code: windows::core::HRESULT) -> bool {
    [
        DXGI_ERROR_DEVICE_REMOVED,
        DXGI_ERROR_DEVICE_HUNG,
        DXGI_ERROR_DEVICE_RESET,
        DXGI_ERROR_DRIVER_INTERNAL_ERROR,
    ]
    .contains(&code)
}

fn os(api: &'static str) -> impl Fn(windows::core::Error) -> LoadError {
    move |e| {
        if is_device_lost(e.code()) {
            LoadError::DeviceLost(e.code().0 as u32)
        } else {
            LoadError::Other(format!("{api}: {e}"))
        }
    }
}

fn missing(what: &str) -> LoadError {
    LoadError::Other(format!("{what}: порожній результат"))
}

/// Навантаження на одній відеокарті.
pub struct GpuLoad {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    fill: ID3D11ComputeShader,
    transform: ID3D11ComputeShader,
    constants: ID3D11Buffer,
    srv: [ID3D11ShaderResourceView; 2],
    uav: [ID3D11UnorderedAccessView; 2],
    digest: ID3D11Buffer,
    digest_uav: ID3D11UnorderedAccessView,
    staging: ID3D11Buffer,
    query: ID3D11Query,
    groups: u32,
    elements: u32,
    params: GpuParams,
    adapter: String,
    vram_mb: u64,
    tested_mb: u64,
}

/// Відеокарта з найбільшою власною пам'яттю (не вбудований і не програмний адаптер).
fn best_adapter() -> Result<(IDXGIAdapter1, String, u64), LoadError> {
    // SAFETY: фабрика створюється без вхідних вказівників.
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(os("CreateDXGIFactory1"))?;
    let mut best: Option<(IDXGIAdapter1, String, u64)> = None;
    for i in 0.. {
        // SAFETY: звичайний перебір; помилка означає кінець списку.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        // SAFETY: адаптер щойно отримано.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
            continue;
        }
        let bytes = desc.DedicatedVideoMemory as u64;
        if best.as_ref().is_none_or(|b| bytes / (1024 * 1024) > b.2) {
            let end = desc.Description.iter().position(|c| *c == 0);
            let name = String::from_utf16_lossy(
                &desc.Description[..end.unwrap_or(desc.Description.len())],
            );
            best = Some((adapter, name, bytes / (1024 * 1024)));
        }
    }
    best.ok_or_else(|| LoadError::Other("апаратної відеокарти не знайдено".into()))
}

fn compile(entry: PCSTR) -> Result<Vec<u8>, LoadError> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: джерело — статичний рядок (вказівник і довжина узгоджені), результати пишуться в
    // локальні Option.
    let result = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            PCSTR::null(),
            None,
            None,
            entry,
            windows::core::s!("cs_5_0"),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    let bytes = |b: &ID3DBlob| {
        // SAFETY: буфер належить blob і читається, поки blob живий.
        unsafe {
            std::slice::from_raw_parts(b.GetBufferPointer().cast::<u8>(), b.GetBufferSize())
                .to_vec()
        }
    };
    match (result, code) {
        (Ok(()), Some(code)) => Ok(bytes(&code)),
        (r, _) => {
            let message = errors
                .as_ref()
                .map(|e| String::from_utf8_lossy(&bytes(e)).trim().to_owned())
                .filter(|m| !m.is_empty())
                .or_else(|| r.err().map(|e| e.message()))
                .unwrap_or_default();
            Err(LoadError::Other(format!("шейдер: {message}")))
        }
    }
}

impl GpuLoad {
    /// Створює пристрій на найпотужнішій відеокарті й виділяє відеопам'ять.
    pub fn new(params: &GpuParams) -> Result<Self, LoadError> {
        let (adapter1, adapter, vram_mb) = best_adapter()?;
        let adapter_base: IDXGIAdapter = adapter1.cast().map_err(os("IDXGIAdapter"))?;
        let mut device = None;
        let mut context = None;
        // SAFETY: усі вказівники — на локальні Option, які заповнює функція.
        unsafe {
            D3D11CreateDevice(
                &adapter_base,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(os("D3D11CreateDevice"))?;
        let (Some(device), Some(context)) = (device, context) else {
            return Err(missing("D3D11CreateDevice"));
        };

        let shader = |entry: PCSTR| -> Result<ID3D11ComputeShader, LoadError> {
            let code = compile(entry)?;
            let mut out = None;
            // SAFETY: байткод щойно скомпільований для цієї стадії.
            unsafe { device.CreateComputeShader(&code, None, Some(&mut out)) }
                .map_err(os("CreateComputeShader"))?;
            out.ok_or_else(|| missing("CreateComputeShader"))
        };
        let fill = shader(windows::core::s!("fill"))?;
        let transform = shader(windows::core::s!("transform"))?;

        let buffer = |desc: &D3D11_BUFFER_DESC| -> Result<ID3D11Buffer, LoadError> {
            let mut out = None;
            // SAFETY: опис живе на час виклику, початкових даних немає.
            unsafe { device.CreateBuffer(desc, None, Some(&mut out)) }
                .map_err(os("CreateBuffer"))?;
            out.ok_or_else(|| missing("CreateBuffer"))
        };
        let structured = |bytes: u32, stride: u32| D3D11_BUFFER_DESC {
            ByteWidth: bytes,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_UNORDERED_ACCESS.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: stride,
        };

        // Чверть відеопам'яті на буфер (стільки D3D11 гарантує одному ресурсу), кратно групі.
        let mut mb = params.buffer_mb.min(vram_mb / 4).max(16);
        let (a, b, tested_bytes) = loop {
            let bytes = u32::try_from(mb * 1024 * 1024).unwrap_or(u32::MAX - 4095);
            let bytes = bytes / (ELEMENT_BYTES * GROUP_THREADS) * (ELEMENT_BYTES * GROUP_THREADS);
            match (
                buffer(&structured(bytes, ELEMENT_BYTES)),
                buffer(&structured(bytes, ELEMENT_BYTES)),
            ) {
                (Ok(a), Ok(b)) => break (a, b, bytes),
                (Err(LoadError::DeviceLost(c)), _) | (_, Err(LoadError::DeviceLost(c))) => {
                    return Err(LoadError::DeviceLost(c))
                }
                // Пам'ять зайнята іншими програмами: тест менший, але чесно показує скільки.
                _ if mb > 64 => mb /= 2,
                (Err(e), _) | (_, Err(e)) => return Err(e),
            }
        };
        let digest = buffer(&structured(4 * DIGEST_WORDS as u32, 4))?;
        let staging = buffer(&D3D11_BUFFER_DESC {
            ByteWidth: 4 * DIGEST_WORDS as u32,
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        })?;
        let constants = buffer(&D3D11_BUFFER_DESC {
            ByteWidth: 16,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        })?;

        let srv_of = |b: &ID3D11Buffer| -> Result<ID3D11ShaderResourceView, LoadError> {
            let mut out = None;
            // SAFETY: буфер дійсний; без опису вид охоплює весь структурований буфер.
            unsafe { device.CreateShaderResourceView(b, None, Some(&mut out)) }
                .map_err(os("CreateShaderResourceView"))?;
            out.ok_or_else(|| missing("CreateShaderResourceView"))
        };
        let uav_of = |b: &ID3D11Buffer| -> Result<ID3D11UnorderedAccessView, LoadError> {
            let mut out = None;
            // SAFETY: буфер дійсний; без опису вид охоплює весь структурований буфер.
            unsafe { device.CreateUnorderedAccessView(b, None, Some(&mut out)) }
                .map_err(os("CreateUnorderedAccessView"))?;
            out.ok_or_else(|| missing("CreateUnorderedAccessView"))
        };
        let srv = [srv_of(&a)?, srv_of(&b)?];
        let uav = [uav_of(&a)?, uav_of(&b)?];
        let digest_uav = uav_of(&digest)?;

        let mut query = None;
        // SAFETY: опис живе на час виклику.
        unsafe {
            device.CreateQuery(
                &D3D11_QUERY_DESC {
                    Query: D3D11_QUERY_EVENT,
                    MiscFlags: 0,
                },
                Some(&mut query),
            )
        }
        .map_err(os("CreateQuery"))?;
        let query = query.ok_or_else(|| missing("CreateQuery"))?;

        Ok(Self {
            fill,
            transform,
            constants,
            srv,
            uav,
            digest,
            digest_uav,
            staging,
            query,
            groups: MIN_GROUPS,
            elements: tested_bytes / ELEMENT_BYTES,
            params: params.clone(),
            adapter,
            vram_mb,
            tested_mb: u64::from(tested_bytes) * 2 / (1024 * 1024),
            device,
            context,
        })
    }

    /// Назва відеокарти.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Обсяг відеопам'яті, МБ.
    pub fn vram_mb(&self) -> u64 {
        self.vram_mb
    }

    /// Скільки відеопам'яті перевіряє тест, МБ.
    pub fn tested_mb(&self) -> u64 {
        self.tested_mb
    }

    /// Чекає, поки GPU виконає все надіслане.
    fn wait(&self) -> Result<(), LoadError> {
        // SAFETY: запит створено на цьому пристрої, контекст використовує один потік.
        unsafe {
            self.context.End(&self.query);
            self.context.Flush();
        }
        let started = Instant::now();
        loop {
            let mut done = 0i32;
            // SAFETY: `done` живе на час виклику; розмір відповідає типу результату запиту (BOOL).
            let polled = unsafe {
                self.context.GetData(
                    &self.query,
                    Some((&raw mut done).cast::<c_void>()),
                    size_of::<i32>() as u32,
                    0,
                )
            };
            polled.map_err(os("GetData"))?;
            if done != 0 {
                return Ok(());
            }
            // SAFETY: запит стану пристрою без вказівників.
            unsafe { self.device.GetDeviceRemovedReason() }
                .map_err(os("GetDeviceRemovedReason"))?;
            if started.elapsed() > WAIT_LIMIT {
                return Err(LoadError::Other(
                    "GPU не завершив порцію роботи за 30 с".into(),
                ));
            }
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    /// Прохід над усім буфером короткими порціями. `from` — вхід (індекс буфера), `to` — вихід.
    /// `Ok(false)` — попросили зупинитись.
    fn pass(
        &mut self,
        shader: &ID3D11ComputeShader,
        seed: u32,
        from: usize,
        to: usize,
        stop: &mut dyn FnMut() -> bool,
    ) -> Result<bool, LoadError> {
        let uavs = [Some(self.uav[to].clone()), Some(self.digest_uav.clone())];
        let none_uavs: [Option<ID3D11UnorderedAccessView>; 2] = [None, None];
        let none_srv: [Option<ID3D11ShaderResourceView>; 1] = [None];
        // SAFETY: усі види створені на цьому пристрої й живуть довше за виклики; контекст
        // використовує один потік; вхід і вихід — різні буфери.
        unsafe {
            self.context.CSSetShader(shader, None);
            self.context
                .CSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context
                .CSSetShaderResources(0, Some(&[Some(self.srv[from].clone())]));
            self.context
                .CSSetUnorderedAccessViews(0, 2, Some(uavs.as_ptr()), None);
        }
        let total_groups = self.elements / GROUP_THREADS;
        let mut group = 0u32;
        let mut finished = true;
        while group < total_groups {
            if stop() {
                finished = false;
                break;
            }
            let groups = self.groups.min(total_groups - group);
            let values = [
                seed,
                group * GROUP_THREADS,
                self.elements,
                self.params.inner,
            ];
            let started = Instant::now();
            // SAFETY: буфер констант на 16 байт, `values` — стільки ж; порція обмежена груп і
            // не виходить за межі буфера (offset + groups·256 ≤ elements).
            unsafe {
                self.context.UpdateSubresource(
                    &self.constants,
                    0,
                    None,
                    values.as_ptr().cast(),
                    0,
                    0,
                );
                self.context.Dispatch(groups, 1, 1);
            }
            self.wait()?;
            group += groups;
            self.adapt(started.elapsed(), groups);
        }
        // SAFETY: відв'язуємо, щоб наступний прохід міг використати буфери навпаки.
        unsafe {
            self.context
                .CSSetUnorderedAccessViews(0, 2, Some(none_uavs.as_ptr()), None);
            self.context.CSSetShaderResources(0, Some(&none_srv));
        }
        Ok(finished)
    }

    /// Підлаштовує порцію під бажану тривалість, не більш ніж удвічі за раз.
    fn adapt(&mut self, took: Duration, groups: u32) {
        if groups < self.groups {
            return;
        }
        let ratio = self.params.slice.as_secs_f64() / took.as_secs_f64().max(1e-4);
        self.groups = next_groups(self.groups, ratio);
    }
}

/// Наступний розмір порції: не більш ніж удвічі за раз і в межах, які приймає Direct3D 11.
fn next_groups(groups: u32, ratio: f64) -> u32 {
    let next = f64::from(groups) * ratio.clamp(0.5, 2.0);
    (next as u32).clamp(MIN_GROUPS, MAX_GROUPS)
}

/// Відбиток порожній, тобто GPU не виконав роботу (інакше порожні результати «збігалися» б).
fn is_empty(digest: &Digest) -> bool {
    digest.iter().all(|w| *w == 0)
}

impl Load for GpuLoad {
    fn round(
        &mut self,
        seed: u32,
        stop: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Digest>, LoadError> {
        // SAFETY: вид створено на цьому пристрої.
        unsafe {
            self.context
                .ClearUnorderedAccessViewUint(&self.digest_uav, &[0u32; 4]);
        }
        let (fill, transform) = (self.fill.clone(), self.transform.clone());
        // Вхід для заповнення не читається, але слот шейдера не має бути порожнім.
        if !self.pass(&fill, seed, 1, 0, stop)? || !self.pass(&transform, seed, 0, 1, stop)? {
            return Ok(None);
        }
        // Другий прохід читає вже записане: перевіряє й пам'ять, і перший прохід. Відбиток рахує
        // сума обох.
        if !self.pass(&transform, seed, 1, 0, stop)? {
            return Ok(None);
        }
        // SAFETY: обидва буфери однакового розміру, контекст використовує один потік.
        unsafe { self.context.CopyResource(&self.staging, &self.digest) };
        self.wait()?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: staging-буфер створено для читання процесором; `mapped` живе на час виклику.
        unsafe {
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .map_err(os("Map"))?;
        let mut out: Digest = [0; DIGEST_WORDS];
        // SAFETY: успішний Map дає щонайменше DIGEST_WORDS слів; область не перетинається з `out`.
        unsafe {
            std::ptr::copy_nonoverlapping(
                mapped.pData.cast::<u32>(),
                out.as_mut_ptr(),
                DIGEST_WORDS,
            );
            self.context.Unmap(&self.staging, 0);
        }
        if is_empty(&out) {
            return Err(LoadError::Other(
                "GPU не виконав обчислення: порожній результат".into(),
            ));
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{Checker, Observed};
    use crate::worker::SEEDS;

    /// Маленьке завдання на справжній відеокарті. Без апаратної відеокарти тест нічого не робить.
    fn small() -> Option<GpuLoad> {
        match GpuLoad::new(&GpuParams {
            buffer_mb: 16,
            inner: 8,
            slice: Duration::from_millis(20),
        }) {
            Ok(g) => Some(g),
            Err(e) => {
                println!("немає пристрою: {e:?}");
                None
            }
        }
    }

    #[test]
    fn result_repeats_exactly_and_depends_on_the_task() {
        let Some(mut g) = small() else { return };
        let mut c = Checker::default();
        let mut never = || false;
        let d0 = g
            .round(SEEDS[0], &mut never)
            .expect("завдання")
            .expect("завершено");
        let d1 = g
            .round(SEEDS[1], &mut never)
            .expect("завдання")
            .expect("завершено");
        assert_ne!(d0, d1, "різні завдання дають різний результат");
        assert!(
            d0.iter().filter(|w| **w != 0).count() >= DIGEST_WORDS - 1,
            "{d0:?}"
        );
        assert_eq!(c.observe(SEEDS[0], &d0), Observed::Reference);
        assert_eq!(c.observe(SEEDS[1], &d1), Observed::Reference);
        for _ in 0..3 {
            for (seed, reference) in [(SEEDS[0], d0), (SEEDS[1], d1)] {
                let d = g
                    .round(seed, &mut never)
                    .expect("завдання")
                    .expect("завершено");
                assert_eq!(d, reference, "той самий результат до біта");
            }
        }
    }

    #[test]
    fn slices_never_exceed_what_direct3d_accepts() {
        assert_eq!(next_groups(100, 2.0), 200);
        assert_eq!(next_groups(100, 100.0), 200, "не більш ніж удвічі");
        assert_eq!(next_groups(100, 0.01), MIN_GROUPS.max(50));
        assert_eq!(next_groups(MIN_GROUPS, 0.01), MIN_GROUPS);
        assert_eq!(next_groups(MAX_GROUPS, 2.0), MAX_GROUPS);
        assert!(is_empty(&[0; DIGEST_WORDS]));
        let mut one = [0; DIGEST_WORDS];
        one[9] = 1;
        assert!(!is_empty(&one));
    }

    /// Буфер більший за одну найбільшу порцію: уся робота має справді виконуватись, і результат
    /// не може залежати від того, на які порції її поділено.
    #[test]
    fn large_buffer_is_fully_processed_whatever_the_slice_size() {
        let Ok(mut g) = GpuLoad::new(&GpuParams {
            buffer_mb: 320,
            inner: 1,
            slice: Duration::from_millis(50),
        }) else {
            return;
        };
        let total_groups = g.elements / GROUP_THREADS;
        if total_groups <= MAX_GROUPS {
            // Замало відеопам'яті для цієї перевірки.
            return;
        }
        let mut never = || false;
        let small_slices = {
            g.params.slice = Duration::from_micros(1);
            g.groups = MIN_GROUPS * 4;
            g.round(SEEDS[2], &mut never)
                .expect("завдання")
                .expect("завершено")
        };
        assert!(small_slices.iter().filter(|w| **w != 0).count() >= DIGEST_WORDS - 1);
        g.params.slice = Duration::from_secs(10);
        g.groups = MAX_GROUPS;
        let big_slices = g
            .round(SEEDS[2], &mut never)
            .expect("завдання")
            .expect("завершено");
        assert_eq!(small_slices, big_slices);
    }

    #[test]
    fn stop_request_interrupts_a_round() {
        let Some(mut g) = small() else { return };
        let mut asked = 0;
        let mut stop = || {
            asked += 1;
            asked > 2
        };
        assert_eq!(g.round(SEEDS[0], &mut stop).expect("завдання"), None);
    }
}
