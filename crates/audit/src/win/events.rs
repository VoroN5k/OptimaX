//! Журнал подій System через wevtapi (лише читання): TDR (`Display`, 4101) і `WHEA-Logger`.
//!
//! Перевірено на Windows 11 Pro 26300 (2026-10-30): XPath із `timediff` приймається журналом,
//! класичні події (як `Display`) віддають рядки вставки через `EvtRenderContextUser`.

use crate::model::SystemEvent;
use crate::rules::{DISPLAY_PROVIDER, TDR_EVENT_ID, WHEA_PROVIDER};
use optimax_core::{CivilDate, CoreError};
use windows::core::{HRESULT, HSTRING};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS};
use windows::Win32::System::EventLog::{
    EvtClose, EvtCreateRenderContext, EvtNext, EvtQuery, EvtQueryChannelPath,
    EvtQueryReverseDirection, EvtRender, EvtRenderContextSystem, EvtRenderContextUser,
    EvtRenderEventValues, EvtSystemEventID, EvtSystemLevel, EvtSystemProviderName,
    EvtSystemTimeCreated, EvtVarTypeByte, EvtVarTypeFileTime, EvtVarTypeString, EvtVarTypeUInt16,
    EVT_HANDLE, EVT_VARIANT, EVT_VARIANT_TYPE_ARRAY, EVT_VARIANT_TYPE_MASK,
};

/// Скільки подій просимо за один виклик `EvtNext`.
const BATCH: usize = 64;
/// Більше подій не читаємо (сотні корегованих WHEA за місяць — уже достатній доказ).
const MAX_EVENTS: usize = 2000;
/// Тайм-аут одного `EvtNext`, мс (локальний журнал відповідає миттєво).
const NEXT_TIMEOUT_MS: u32 = 5000;

/// Закриває дескриптор wevtapi.
struct Handle(EVT_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: дескриптор отримано від wevtapi і закривається рівно один раз.
        let _ = unsafe { EvtClose(self.0) };
    }
}

fn os(api: &'static str, e: &windows::core::Error) -> CoreError {
    CoreError::Os {
        api,
        code: e.code().0 as u32,
    }
}

/// Події TDR і WHEA за останні `days` днів, від нових до старих.
pub(super) fn hardware(days: u32) -> Result<Vec<SystemEvent>, CoreError> {
    let ms = u64::from(days) * 86_400_000;
    query(&format!(
        "*[System[(Provider[@Name='{WHEA_PROVIDER}'] or (Provider[@Name='{DISPLAY_PROVIDER}'] \
         and EventID={TDR_EVENT_ID})) and TimeCreated[timediff(@SystemTime) <= {ms}]]]"
    ))
}

/// Виконує XPath-запит до журналу System.
fn query(xpath: &str) -> Result<Vec<SystemEvent>, CoreError> {
    let path = HSTRING::from("System");
    let xpath = HSTRING::from(xpath);
    let flags = EvtQueryChannelPath.0 | EvtQueryReverseDirection.0;
    // SAFETY: рядки живуть до кінця виклику; сесія — локальна (None).
    let results =
        unsafe { EvtQuery(None, &path, &xpath, flags) }.map_err(|e| os("EvtQuery", &e))?;
    let results = Handle(results);
    // SAFETY: без шляхів значень; прапорець вибирає набір системних властивостей.
    let sys = unsafe { EvtCreateRenderContext(None, EvtRenderContextSystem.0) }
        .map_err(|e| os("EvtCreateRenderContext", &e))?;
    let sys = Handle(sys);
    // SAFETY: те саме для даних події (EventData / UserData).
    let user = unsafe { EvtCreateRenderContext(None, EvtRenderContextUser.0) }
        .map_err(|e| os("EvtCreateRenderContext", &e))?;
    let user = Handle(user);

    let mut out = Vec::new();
    let mut batch = [0isize; BATCH];
    while out.len() < MAX_EVENTS {
        let mut returned = 0u32;
        // SAFETY: масив на BATCH дескрипторів (функція пише не більше), `returned` — валідний.
        let next = unsafe { EvtNext(results.0, &mut batch, NEXT_TIMEOUT_MS, 0, &mut returned) };
        if let Err(e) = next {
            if e.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) {
                break;
            }
            return Err(os("EvtNext", &e));
        }
        let got = usize::try_from(returned).unwrap_or(0).min(BATCH);
        // Кожен дескриптор закривається через Handle, навіть якщо подію не розібрали.
        let events: Vec<Handle> = batch[..got]
            .iter()
            .map(|h| Handle(EVT_HANDLE(*h)))
            .collect();
        for ev in &events {
            if let Some(e) = event(&sys, &user, ev)? {
                out.push(e);
            }
        }
    }
    out.truncate(MAX_EVENTS);
    Ok(out)
}

fn event(sys: &Handle, user: &Handle, ev: &Handle) -> Result<Option<SystemEvent>, CoreError> {
    let sys_buf = render(sys, ev)?;
    let props = variants(&sys_buf);
    let get = |id: i32| usize::try_from(id).ok().and_then(|i| props.get(i));
    let (Some(provider), Some(id), Some(time)) = (
        get(EvtSystemProviderName.0).and_then(string),
        get(EvtSystemEventID.0).and_then(uint16),
        get(EvtSystemTimeCreated.0).and_then(filetime),
    ) else {
        return Ok(None);
    };
    let level = get(EvtSystemLevel.0).and_then(byte).unwrap_or(0);
    let user_buf = render(user, ev)?;
    let data = variants(&user_buf)
        .iter()
        .map(|v| string(v).unwrap_or_default())
        .collect();
    Ok(Some(SystemEvent {
        provider,
        id,
        level,
        date: CivilDate::from_filetime(time),
        data,
    }))
}

/// Буфер із масивом `EVT_VARIANT` (вирівняний на 8 байт) і їх кількість.
struct Rendered {
    buf: Vec<u64>,
    count: usize,
}

fn render(ctx: &Handle, ev: &Handle) -> Result<Rendered, CoreError> {
    let mut used = 0u32;
    let mut count = 0u32;
    // SAFETY: буфера немає (розмір 0) — функція лише повертає потрібний розмір у `used`.
    let probe = unsafe {
        EvtRender(
            Some(ctx.0),
            ev.0,
            EvtRenderEventValues.0,
            0,
            None,
            &mut used,
            &mut count,
        )
    };
    match probe {
        Ok(()) => {
            return Ok(Rendered {
                buf: Vec::new(),
                count: 0,
            })
        }
        Err(e) if e.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => {}
        Err(e) => return Err(os("EvtRender", &e)),
    }
    let words = usize::try_from(used).unwrap_or(0).div_ceil(8);
    let mut buf = vec![0u64; words];
    let size = u32::try_from(words * 8)
        .map_err(|_| CoreError::InvalidData("EvtRender: завеликий буфер".into()))?;
    // SAFETY: `buf` має рівно `size` байт і вирівняний на 8 (u64) — придатний для EVT_VARIANT.
    unsafe {
        EvtRender(
            Some(ctx.0),
            ev.0,
            EvtRenderEventValues.0,
            size,
            Some(buf.as_mut_ptr().cast()),
            &mut used,
            &mut count,
        )
    }
    .map_err(|e| os("EvtRender", &e))?;
    Ok(Rendered {
        buf,
        count: usize::try_from(count).unwrap_or(0),
    })
}

fn variants(r: &Rendered) -> &[EVT_VARIANT] {
    let max = r.buf.len() * 8 / std::mem::size_of::<EVT_VARIANT>();
    if max == 0 {
        return &[];
    }
    // SAFETY: EvtRender записав `count` EVT_VARIANT на початок буфера (вирівняного на 8);
    // кількість обмежено розміром буфера; зріз живе не довше за `r`.
    unsafe { std::slice::from_raw_parts(r.buf.as_ptr().cast::<EVT_VARIANT>(), r.count.min(max)) }
}

fn kind(v: &EVT_VARIANT) -> Option<i32> {
    if v.Type & EVT_VARIANT_TYPE_ARRAY != 0 {
        return None;
    }
    i32::try_from(v.Type & EVT_VARIANT_TYPE_MASK).ok()
}

fn string(v: &EVT_VARIANT) -> Option<String> {
    if kind(v)? != EvtVarTypeString.0 {
        return None;
    }
    // SAFETY: тип String — активне поле StringVal.
    let p = unsafe { v.Anonymous.StringVal };
    if p.is_null() {
        return None;
    }
    // SAFETY: нуль-термінований рядок лежить у буфері EvtRender, який ще живий.
    unsafe { p.to_string() }.ok()
}

fn uint16(v: &EVT_VARIANT) -> Option<u16> {
    if kind(v)? != EvtVarTypeUInt16.0 {
        return None;
    }
    // SAFETY: тип UInt16 — активне поле UInt16Val.
    Some(unsafe { v.Anonymous.UInt16Val })
}

fn byte(v: &EVT_VARIANT) -> Option<u8> {
    if kind(v)? != EvtVarTypeByte.0 {
        return None;
    }
    // SAFETY: тип Byte — активне поле ByteVal.
    Some(unsafe { v.Anonymous.ByteVal })
}

fn filetime(v: &EVT_VARIANT) -> Option<u64> {
    if kind(v)? != EvtVarTypeFileTime.0 {
        return None;
    }
    // SAFETY: тип FileTime — активне поле FileTimeVal.
    Some(unsafe { v.Anonymous.FileTimeVal })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Реальний журнал цієї машини (лише читання). Подій може не бути — тоді перевіряємо,
    /// що запит узагалі виконується; якщо є — що поля розібрано.
    #[test]
    fn reads_real_system_log() {
        let events = query(
            "*[System[Provider[@Name='Display'] and TimeCreated[timediff(@SystemTime) <= 31536000000]]]",
        )
        .unwrap();
        for e in &events {
            assert_eq!(e.provider, "Display");
            assert!(e.id > 0);
            assert!(e.date.year >= 2020);
        }
        // Події з даними: Service Control Manager пише рядки вставки (ім'я служби, стан).
        let scm = query("*[System[Provider[@Name='Service Control Manager']]]").unwrap();
        if let Some(e) = scm.first() {
            assert!(!e.data.is_empty(), "{e:?}");
        }
        hardware(30).unwrap();
    }
}
