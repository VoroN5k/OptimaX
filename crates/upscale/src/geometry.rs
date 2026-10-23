//! Геометрія апскейлу: яку частину кадру захоплення брати і куди її намалювати на екрані.
//! Чисті функції без системних викликів — покриті тестами.

use serde::{Deserialize, Serialize};

/// Розмір у фізичних пікселях.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Size {
    /// Ширина.
    pub width: u32,
    /// Висота.
    pub height: u32,
}

impl Size {
    /// Розмір `width × height`.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Чи нульова площа.
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Прямокутник у фізичних пікселях: лівий верхній кут і розмір.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Rect {
    /// Лівий край.
    pub x: i32,
    /// Верхній край.
    pub y: i32,
    /// Ширина.
    pub width: u32,
    /// Висота.
    pub height: u32,
}

impl Rect {
    /// Прямокутник з кутом `(x, y)` і розміром `width × height`.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Прямокутник за краями `left, top, right, bottom` (як `RECT` у WinAPI).
    /// Вивернутий прямокутник дає нульовий розмір.
    pub fn from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            x: left,
            y: top,
            width: span(left, right),
            height: span(top, bottom),
        }
    }

    /// Розмір.
    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }
}

fn span(from: i32, to: i32) -> u32 {
    u32::try_from(i64::from(to) - i64::from(from)).unwrap_or(0)
}

/// Як вписати кадр гри в екран.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    /// Зберегти пропорції, найбільший розмір, що вміщується (чорні смуги по боках).
    #[default]
    Aspect,
    /// Лише цілий множник (×2, ×3…) — піксель у піксель, без розмиття; більше смуг.
    Integer,
    /// Розтягнути на весь екран, ігноруючи пропорції.
    Stretch,
}

/// Яку частину кадру захоплення займає клієнтська область вікна (без рамки й заголовка).
///
/// Кадр Windows Graphics Capture відповідає видимим межам вікна `frame_bounds`
/// (`DWMWA_EXTENDED_FRAME_BOUNDS`), а розмір кадру `content` може на мить відставати від
/// вікна під час зміни розміру — тому результат обрізається по кадру.
/// `None`, якщо клієнтська область не перетинається з кадром.
pub fn client_crop(frame_bounds: Rect, content: Size, client: Rect) -> Option<Rect> {
    let left = (i64::from(client.x) - i64::from(frame_bounds.x)).max(0);
    let top = (i64::from(client.y) - i64::from(frame_bounds.y)).max(0);
    let right = (i64::from(client.x) + i64::from(client.width) - i64::from(frame_bounds.x))
        .min(i64::from(content.width));
    let bottom = (i64::from(client.y) + i64::from(client.height) - i64::from(frame_bounds.y))
        .min(i64::from(content.height));
    if right <= left || bottom <= top {
        return None;
    }
    Some(Rect::new(
        i32::try_from(left).ok()?,
        i32::try_from(top).ok()?,
        u32::try_from(right - left).ok()?,
        u32::try_from(bottom - top).ok()?,
    ))
}

/// Куди намалювати кадр розміру `source` на виході розміру `output` (координати виходу).
/// Результат завжди по центру й не виходить за межі виходу.
pub fn dest_rect(source: Size, output: Size, fit: FitMode) -> Rect {
    if source.is_empty() || output.is_empty() {
        return Rect::new(0, 0, 0, 0);
    }
    let (w, h) = match fit {
        FitMode::Stretch => (output.width, output.height),
        FitMode::Integer => match integer_factor(source, output) {
            0 => aspect_size(source, output),
            k => (source.width * k, source.height * k),
        },
        FitMode::Aspect => aspect_size(source, output),
    };
    centered(w, h, output)
}

/// Найбільший цілий множник, з яким `source` вміщується в `output` (0 — не вміщується навіть 1:1).
pub fn integer_factor(source: Size, output: Size) -> u32 {
    if source.is_empty() {
        return 0;
    }
    (output.width / source.width).min(output.height / source.height)
}

/// Чи є сенс масштабувати: кадр менший за вихід хоча б по одній осі.
pub fn needs_upscale(source: Size, output: Size) -> bool {
    !source.is_empty() && (source.width < output.width || source.height < output.height)
}

/// Фактичний множник масштабу (за шириною), напр. 1.5 для 1280 → 1920.
pub fn scale_factor(source: Size, dest: Rect) -> f64 {
    if source.width == 0 {
        return 0.0;
    }
    f64::from(dest.width) / f64::from(source.width)
}

fn aspect_size(source: Size, output: Size) -> (u32, u32) {
    // Порівнюємо пропорції в цілих (u64 — без переповнення): ширший за екран кадр
    // упирається в ширину, вищий — у висоту.
    let sw = u64::from(source.width);
    let sh = u64::from(source.height);
    let ow = u64::from(output.width);
    let oh = u64::from(output.height);
    if sw * oh >= sh * ow {
        let h = (sh * ow + sw / 2) / sw;
        (output.width, clamp_u32(h, output.height))
    } else {
        let w = (sw * oh + sh / 2) / sh;
        (clamp_u32(w, output.width), output.height)
    }
}

fn clamp_u32(v: u64, max: u32) -> u32 {
    u32::try_from(v).unwrap_or(max).clamp(1, max)
}

fn centered(w: u32, h: u32, output: Size) -> Rect {
    let w = w.min(output.width);
    let h = h.min(output.height);
    let x = (output.width - w) / 2;
    let y = (output.height - h) / 2;
    Rect::new(
        i32::try_from(x).unwrap_or(0),
        i32::try_from(y).unwrap_or(0),
        w,
        h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const QHD: Size = Size::new(2560, 1440);

    #[test]
    fn crop_skips_title_bar_and_borders() {
        // Вікно 1296×759 з рамкою 8 px і заголовком 31 px; клієнт 1280×720.
        let frame = Rect::new(100, 100, 1296, 759);
        let client = Rect::new(108, 131, 1280, 720);
        let crop = client_crop(frame, Size::new(1296, 759), client);
        assert_eq!(crop, Some(Rect::new(8, 31, 1280, 720)));
    }

    #[test]
    fn crop_of_borderless_window_is_whole_frame() {
        let frame = Rect::new(0, 0, 1920, 1080);
        let crop = client_crop(frame, Size::new(1920, 1080), frame);
        assert_eq!(crop, Some(frame));
    }

    #[test]
    fn crop_is_clamped_to_lagging_frame() {
        // Вікно вже збільшили, а кадр ще старого розміру.
        let frame = Rect::new(0, 0, 1600, 900);
        let crop = client_crop(frame, Size::new(1280, 720), frame);
        assert_eq!(crop, Some(Rect::new(0, 0, 1280, 720)));
    }

    #[test]
    fn crop_outside_frame_is_none() {
        let frame = Rect::new(0, 0, 800, 600);
        let client = Rect::new(900, 0, 100, 100);
        assert_eq!(client_crop(frame, Size::new(800, 600), client), None);
        assert_eq!(
            client_crop(frame, Size::new(800, 600), Rect::new(10, 10, 0, 0)),
            None
        );
    }

    #[test]
    fn rect_from_edges() {
        assert_eq!(
            Rect::from_edges(-1920, 0, 0, 1080),
            Rect::new(-1920, 0, 1920, 1080)
        );
        assert_eq!(Rect::from_edges(10, 10, 5, 5).size(), Size::new(0, 0));
    }

    #[test]
    fn aspect_same_ratio_fills_screen() {
        let r = dest_rect(Size::new(1280, 720), QHD, FitMode::Aspect);
        assert_eq!(r, Rect::new(0, 0, 2560, 1440));
    }

    #[test]
    fn aspect_4_3_gets_side_bars() {
        let r = dest_rect(Size::new(1024, 768), QHD, FitMode::Aspect);
        assert_eq!(r, Rect::new(320, 0, 1920, 1440));
    }

    #[test]
    fn aspect_ultrawide_gets_top_bars() {
        let r = dest_rect(
            Size::new(2560, 1080),
            Size::new(1920, 1080),
            FitMode::Aspect,
        );
        assert_eq!(r.width, 1920);
        assert_eq!(r.height, 810);
        assert_eq!(r.y, 135);
    }

    #[test]
    fn integer_uses_largest_whole_factor() {
        // 1280×720 на 1440p: ×2 рівно.
        assert_eq!(
            dest_rect(Size::new(1280, 720), QHD, FitMode::Integer),
            Rect::new(0, 0, 2560, 1440)
        );
        // 1600×900 на 1440p: ×1.6 → цілий лише ×1, по центру.
        assert_eq!(
            dest_rect(Size::new(1600, 900), QHD, FitMode::Integer),
            Rect::new(480, 270, 1600, 900)
        );
        assert_eq!(integer_factor(Size::new(640, 360), QHD), 4);
    }

    #[test]
    fn integer_falls_back_to_aspect_when_source_is_larger() {
        let r = dest_rect(Size::new(3840, 2160), QHD, FitMode::Integer);
        assert_eq!(r, Rect::new(0, 0, 2560, 1440));
    }

    #[test]
    fn stretch_fills_everything() {
        let r = dest_rect(Size::new(1024, 768), QHD, FitMode::Stretch);
        assert_eq!(r, Rect::new(0, 0, 2560, 1440));
    }

    #[test]
    fn empty_sizes_give_empty_rect() {
        assert_eq!(
            dest_rect(Size::new(0, 720), QHD, FitMode::Aspect).size(),
            Size::default()
        );
        assert_eq!(
            dest_rect(Size::new(1280, 720), Size::new(0, 0), FitMode::Stretch).size(),
            Size::default()
        );
        assert_eq!(integer_factor(Size::default(), QHD), 0);
    }

    #[test]
    fn upscale_needed_only_for_smaller_source() {
        assert!(needs_upscale(Size::new(1280, 720), QHD));
        assert!(needs_upscale(Size::new(2560, 1080), QHD));
        assert!(!needs_upscale(QHD, QHD));
        assert!(!needs_upscale(Size::new(3840, 2160), QHD));
        assert!(!needs_upscale(Size::default(), QHD));
    }

    #[test]
    fn scale_factor_by_width() {
        let r = dest_rect(Size::new(1280, 720), Size::new(1920, 1080), FitMode::Aspect);
        assert!((scale_factor(Size::new(1280, 720), r) - 1.5).abs() < 1e-9);
        assert_eq!(scale_factor(Size::default(), r), 0.0);
    }
}
