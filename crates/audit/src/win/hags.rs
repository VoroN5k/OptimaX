//! Реальний стан HAGS за даними драйвера (D3DKMT, WDDM 2.7+).

use crate::devprop::wide_array_to_string;
use crate::model::HagsAdapter;
use optimax_core::CoreError;
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTEnumAdapters2, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTERINFO,
    D3DKMT_ADAPTERREGISTRYINFO, D3DKMT_CLOSEADAPTER, D3DKMT_ENUMADAPTERS2, D3DKMT_QUERYADAPTERINFO,
    KMTQAITYPE_ADAPTERREGISTRYINFO, KMTQAITYPE_WDDM_2_7_CAPS, KMTQUERYADAPTERINFOTYPE,
};

/// Більше адаптерів D3DKMT не перелічує (`MAX_ENUM_ADAPTERS`).
const MAX_ADAPTERS: usize = 16;

/// Біти `D3DKMT_WDDM_2_7_CAPS` (d3dkmthk.h).
const HWSCH_SUPPORTED: u32 = 1 << 0;
const HWSCH_ENABLED: u32 = 1 << 1;
const HWSCH_ENABLED_BY_DEFAULT: u32 = 1 << 2;

/// Закриває дескриптор адаптера, відкритий `D3DKMTEnumAdapters2`.
struct AdapterHandle(u32);

impl Drop for AdapterHandle {
    fn drop(&mut self) {
        let close = D3DKMT_CLOSEADAPTER { hAdapter: self.0 };
        // SAFETY: дескриптор отримано з D3DKMTEnumAdapters2 і закривається рівно один раз.
        let _ = unsafe { D3DKMTCloseAdapter(&close) };
    }
}

/// Запитує в драйвера структуру `T`. `false`, якщо драйвер не підтримує запит.
fn query<T>(adapter: u32, kind: KMTQUERYADAPTERINFOTYPE, out: &mut T) -> bool {
    let Ok(size) = u32::try_from(std::mem::size_of::<T>()) else {
        return false;
    };
    let mut q = D3DKMT_QUERYADAPTERINFO {
        hAdapter: adapter,
        Type: kind,
        pPrivateDriverData: (out as *mut T).cast(),
        PrivateDriverDataSize: size,
    };
    // SAFETY: `out` — валідний буфер рівно `size` байт, тип відповідає `kind` (див. виклики нижче).
    let status = unsafe { D3DKMTQueryAdapterInfo(&mut q) };
    status.0 >= 0
}

pub(super) fn adapters() -> Result<Vec<HagsAdapter>, CoreError> {
    let mut list = [D3DKMT_ADAPTERINFO::default(); MAX_ADAPTERS];
    let mut e = D3DKMT_ENUMADAPTERS2 {
        NumAdapters: MAX_ADAPTERS as u32,
        pAdapters: list.as_mut_ptr(),
    };
    // SAFETY: масив має `NumAdapters` елементів; функція заповнює не більше.
    let status = unsafe { D3DKMTEnumAdapters2(&mut e) };
    if status.0 < 0 {
        return Err(CoreError::Os {
            api: "D3DKMTEnumAdapters2",
            code: status.0 as u32,
        });
    }
    let count = (e.NumAdapters as usize).min(MAX_ADAPTERS);
    // Обгортаємо всі дескриптори одразу, щоб закрити їх навіть при ранньому виході.
    let handles: Vec<AdapterHandle> = list[..count]
        .iter()
        .map(|a| AdapterHandle(a.hAdapter))
        .collect();

    Ok(handles
        .iter()
        .map(|h| {
            // Старі драйвери (до WDDM 2.7) цього запиту не знають — тоді HAGS просто не підтримується.
            let mut caps: u32 = 0;
            if !query(h.0, KMTQAITYPE_WDDM_2_7_CAPS, &mut caps) {
                caps = 0;
            }
            let mut reg = Box::<D3DKMT_ADAPTERREGISTRYINFO>::default();
            let name = if query(h.0, KMTQAITYPE_ADAPTERREGISTRYINFO, reg.as_mut()) {
                wide_array_to_string(&reg.AdapterString)
            } else {
                String::new()
            };
            HagsAdapter {
                name,
                supported: caps & HWSCH_SUPPORTED != 0,
                enabled: caps & HWSCH_ENABLED != 0,
                enabled_by_default: caps & HWSCH_ENABLED_BY_DEFAULT != 0,
            }
        })
        // Безіменний адаптер без підтримки — програмний рендер (Microsoft Basic Render Driver).
        .filter(|a| a.supported || !a.name.is_empty())
        .collect())
}
