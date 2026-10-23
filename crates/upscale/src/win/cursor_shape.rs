//! Поточний системний курсор: позиція, видимість і зображення (через GDI).

use crate::cursor::premultiplied_from_backgrounds;
use crate::geometry::Size;
use core::ffi::c_void;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetObjectW, SelectObject, BITMAP,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, HCURSOR, HICON,
    ICONINFO,
};

/// Стан курсора зараз.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CursorState {
    /// Ідентифікатор форми (змінюється, коли змінюється курсор: стрілка, рука, текст).
    pub shape: isize,
    /// Позиція на робочому столі.
    pub pos: (i32, i32),
}

/// Видимий курсор (`None` — курсора немає або його сховав застосунок, напр. гра в рівні).
/// Наше власне приховування (Magnification API) на цей прапорець не впливає.
pub(super) fn current() -> Option<CursorState> {
    let mut ci = CURSORINFO {
        cbSize: core::mem::size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: локальна структура з правильним cbSize.
    unsafe { GetCursorInfo(&mut ci) }.ok()?;
    if ci.flags.0 & CURSOR_SHOWING.0 == 0 || ci.hCursor.is_invalid() {
        return None;
    }
    Some(CursorState {
        shape: ci.hCursor.0 as isize,
        pos: (ci.ptScreenPos.x, ci.ptScreenPos.y),
    })
}

/// Зображення курсора: BGRA premultiplied, рядки згори вниз.
pub(super) struct CursorImage {
    pub pixels: Vec<u8>,
    pub size: Size,
    pub hotspot: (i32, i32),
}

/// Розмір растра GDI.
fn bitmap_size(bmp: HBITMAP) -> Option<(i32, i32)> {
    let mut info = BITMAP::default();
    // SAFETY: буфер — локальна структура BITMAP точного розміру.
    let n = unsafe {
        GetObjectW(
            HGDIOBJ(bmp.0),
            core::mem::size_of::<BITMAP>() as i32,
            Some((&mut info as *mut BITMAP).cast::<c_void>()),
        )
    };
    (n > 0).then_some((info.bmWidth, info.bmHeight))
}

/// Малює курсор на суцільному тлі `bg` у 32-бітний растр і повертає його пікселі.
fn render_on(cursor: HCURSOR, w: i32, h: i32, bg: u8) -> Option<Vec<u8>> {
    let header = BITMAPINFOHEADER {
        biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        // Від'ємна висота — рядки згори вниз.
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        ..Default::default()
    };
    let len = usize::try_from(w).ok()? * usize::try_from(h).ok()? * 4;
    // SAFETY: DIB-секція створюється з локального опису; `bits` вказує на її пам'ять розміром
    // `len` (32 біти × w × h) і читається лише поки секція жива; DC і об'єкти звільняються тут.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let mut bits: *mut c_void = core::ptr::null_mut();
        let dib = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(dc);
                return None;
            }
        };
        let old = SelectObject(dc, HGDIOBJ(dib.0));
        core::ptr::write_bytes(bits.cast::<u8>(), bg, len);
        let drawn = DrawIconEx(dc, 0, 0, HICON(cursor.0), w, h, 0, None, DI_NORMAL).is_ok();
        let pixels = drawn.then(|| std::slice::from_raw_parts(bits.cast::<u8>(), len).to_vec());
        SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(dc);
        pixels
    }
}

/// Зображення курсора `shape` з прозорістю (з двох рендерів — на чорному й на білому тлі).
pub(super) fn image(shape: isize) -> Option<CursorImage> {
    let cursor = HCURSOR(shape as *mut c_void);
    let mut ii = ICONINFO::default();
    // SAFETY: `cursor` отримано щойно з GetCursorInfo; структура локальна.
    unsafe { GetIconInfo(HICON(cursor.0), &mut ii) }.ok()?;
    // Кольоровий курсор — розмір із кольорового растра; монохромний — маска подвійної висоти.
    let size = if ii.hbmColor.is_invalid() {
        bitmap_size(ii.hbmMask).map(|(w, h)| (w, h / 2))
    } else {
        bitmap_size(ii.hbmColor)
    };
    // SAFETY: GetIconInfo створює копії растрів — їх звільняє викликач.
    unsafe {
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        }
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        }
    }
    let (w, h) = size.filter(|(w, h)| *w > 0 && *h > 0 && *w <= 512 && *h <= 512)?;
    let on_black = render_on(cursor, w, h, 0)?;
    let on_white = render_on(cursor, w, h, 255)?;
    Some(CursorImage {
        pixels: premultiplied_from_backgrounds(&on_black, &on_white),
        size: Size::new(w as u32, h as u32),
        hotspot: (ii.xHotspot as i32, ii.yHotspot as i32),
    })
}
