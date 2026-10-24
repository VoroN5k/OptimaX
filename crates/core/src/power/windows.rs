//! Реалізація [`PowerApi`] через Power API Windows.
#![allow(unsafe_code)]

use super::{format_guid, overlay_guids, parse_guid, PowerApi, PowerScheme};
use crate::error::CoreError;
use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, HLOCAL};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::Power::{
    PowerEnumerate, PowerGetActiveScheme, PowerReadFriendlyName, PowerSetActiveScheme,
    ACCESS_SCHEME,
};
use windows::core::s;

type FnGetOverlay = unsafe extern "system" fn(*mut GUID) -> u32;
type FnSetOverlay = unsafe extern "system" fn(GUID) -> u32;

/// Завантажує покажчики на недокументовані функції з `powrprof.dll`.
/// Повертає `None`, якщо функцій немає (старі збірки Windows).
fn load_overlay_fns() -> Option<(FnGetOverlay, FnSetOverlay)> {
    // SAFETY: powrprof.dll завжди завантажено (ми вже використовуємо PowerGetActiveScheme тощо).
    // GetProcAddress повертає FARPROC = Option<fn() -> isize>; transmute приводить до точного типу.
    unsafe {
        let module = GetModuleHandleA(s!("powrprof.dll")).ok()?;
        let get = GetProcAddress(module, s!("PowerGetEffectiveOverlayScheme"))?;
        let set = GetProcAddress(module, s!("PowerSetActiveOverlayScheme"))?;
        Some((std::mem::transmute(get), std::mem::transmute(set)))
    }
}

/// Схеми живлення Windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinPower;

/// Запобіжник від нескінченного переліку.
const MAX_SCHEMES: u32 = 64;

fn friendly_name(guid: &GUID) -> Option<String> {
    let mut size = 0u32;
    // SAFETY: перший виклик без буфера лише повертає розмір.
    let status = unsafe { PowerReadFriendlyName(None, Some(guid), None, None, None, &mut size) };
    if status != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: буфер має `size` байт.
    let status = unsafe {
        PowerReadFriendlyName(
            None,
            Some(guid),
            None,
            None,
            Some(buf.as_mut_ptr()),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let units: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

impl PowerApi for WinPower {
    fn active_scheme(&self) -> Result<String, CoreError> {
        let mut ptr: *mut GUID = std::ptr::null_mut();
        // SAFETY: функція записує вказівник на GUID, виділений через LocalAlloc.
        let status = unsafe { PowerGetActiveScheme(None, &mut ptr) };
        if status != ERROR_SUCCESS || ptr.is_null() {
            return Err(CoreError::Os {
                api: "PowerGetActiveScheme",
                code: status.0,
            });
        }
        // SAFETY: `ptr` ненульовий і вказує на GUID; копіюємо до звільнення.
        let guid = unsafe { *ptr };
        // SAFETY: пам'ять виділена системою і звільняється один раз.
        let _ = unsafe { LocalFree(Some(HLOCAL(ptr.cast()))) };
        Ok(format_guid(guid.to_u128()))
    }

    fn set_active_scheme(&self, guid: &str) -> Result<(), CoreError> {
        let value = parse_guid(guid)
            .ok_or_else(|| CoreError::InvalidData(format!("некоректний GUID {guid}")))?;
        // Не вмикаємо «наосліп»: схема має бути в системі.
        if !self
            .schemes()?
            .iter()
            .any(|s| s.guid.eq_ignore_ascii_case(guid))
        {
            return Err(CoreError::InvalidData(format!("немає схеми {guid}")));
        }
        let g = GUID::from_u128(value);
        // SAFETY: вказівник на локальний GUID, валідний на час виклику.
        let status = unsafe { PowerSetActiveScheme(None, Some(&g)) };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(CoreError::Os {
                api: "PowerSetActiveScheme",
                code: status.0,
            })
        }
    }

    fn schemes(&self) -> Result<Vec<PowerScheme>, CoreError> {
        let mut out = Vec::new();
        for index in 0..MAX_SCHEMES {
            let mut guid = GUID::zeroed();
            let mut size = std::mem::size_of::<GUID>() as u32;
            // SAFETY: буфер — локальний GUID розміром `size` байт.
            let status = unsafe {
                PowerEnumerate(
                    None,
                    None,
                    None,
                    ACCESS_SCHEME,
                    index,
                    Some((&mut guid as *mut GUID).cast()),
                    &mut size,
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            if status != ERROR_SUCCESS {
                return Err(CoreError::Os {
                    api: "PowerEnumerate",
                    code: status.0,
                });
            }
            out.push(PowerScheme {
                guid: format_guid(guid.to_u128()),
                name: friendly_name(&guid),
            });
        }
        Ok(out)
    }

    fn scheme_name(&self, guid: &str) -> Option<String> {
        friendly_name(&GUID::from_u128(parse_guid(guid)?))
    }

    fn overlay_supported(&self) -> bool {
        load_overlay_fns().is_some()
    }

    fn overlay_scheme(&self) -> Result<Option<String>, CoreError> {
        let (get_fn, _) = load_overlay_fns().ok_or_else(|| {
            CoreError::InvalidData("PowerGetEffectiveOverlayScheme недоступний".into())
        })?;
        let mut guid = GUID::zeroed();
        // SAFETY: get_fn — перевірений покажчик; guid — локальна змінна.
        let status = unsafe { get_fn(&mut guid) };
        if status != ERROR_SUCCESS.0 {
            return Err(CoreError::Os {
                api: "PowerGetEffectiveOverlayScheme",
                code: status,
            });
        }
        if guid.to_u128() == 0 {
            Ok(None)
        } else {
            Ok(Some(format_guid(guid.to_u128())))
        }
    }

    fn set_overlay_scheme(&self, guid: Option<&str>) -> Result<(), CoreError> {
        let (_, set_fn) = load_overlay_fns().ok_or_else(|| {
            CoreError::InvalidData("PowerSetActiveOverlayScheme недоступний".into())
        })?;
        let raw = match guid {
            None => GUID::zeroed(),
            Some(g) => {
                let v = parse_guid(g)
                    .ok_or_else(|| CoreError::InvalidData(format!("некоректний GUID {g}")))?;
                GUID::from_u128(v)
            }
        };
        // SAFETY: set_fn — перевірений покажчик; raw — копія GUID.
        let status = unsafe { set_fn(raw) };
        if status == ERROR_SUCCESS.0 {
            Ok(())
        } else {
            Err(CoreError::Os {
                api: "PowerSetActiveOverlayScheme",
                code: status,
            })
        }
    }
}

// Перевіряємо GUID оверлейних режимів через parse_guid.
const _: () = {
    // Константна перевірка коректності GUID-рядків (компілюється → рядки валідні).
    let _e = overlay_guids::ECONOMY;
    let _b = overlay_guids::BETTER_PERFORMANCE;
    let _p = overlay_guids::BEST_PERFORMANCE;
};
