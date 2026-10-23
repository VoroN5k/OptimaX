//! Власний курсор в оверлеї: форма з прозорістю й позиція у збільшених координатах.
//!
//! Системний курсор під час апскейлу схований, а WGC схованого курсора в кадр не малює,
//! тому OptimaX малює збільшений курсор сам. Чисті функції — покриті тестами.

use crate::geometry::{Rect, Size};

/// Прозорість і колір пікселів курсора з двох рендерів — на чорному й на білому тлі.
/// Для кожного каналу: `на_білому − на_чорному = 255 − α`, а `на_чорному` — це вже колір,
/// помножений на α (premultiplied). Інвертуючі частини (текстовий курсор) стають білими.
/// Вхід і вихід — BGRA по 4 байти на піксель; результат premultiplied.
pub(crate) fn premultiplied_from_backgrounds(on_black: &[u8], on_white: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; on_black.len().min(on_white.len()) / 4 * 4];
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        let b = &on_black[i * 4..i * 4 + 3];
        let w = &on_white[i * 4..i * 4 + 3];
        // Беремо найменшу різницю між каналами — найнадійніша оцінка α для кольорових країв.
        let diff = (0..3)
            .map(|c| i32::from(w[c]) - i32::from(b[c]))
            .min()
            .unwrap_or(255);
        let alpha = (255 - diff).clamp(0, 255);
        if diff < 0 {
            // Інверсія (на білому темніше, ніж на чорному) — показуємо білим непрозорим.
            px.copy_from_slice(&[255, 255, 255, 255]);
            continue;
        }
        for c in 0..3 {
            px[c] = b[c].min(alpha as u8);
        }
        px[3] = alpha as u8;
    }
    out
}

/// Де намалювати курсор в оверлеї.
///
/// `cursor` — позиція курсора на робочому столі; `hotspot` і `size` — гаряча точка й розмір
/// зображення курсора; `client` — клієнтська область гри; `source` — розмір кадру гри;
/// `dest` — куди малюється кадр. Курсор масштабується так само, як картинка гри.
/// `None` — курсор поза вікном гри.
pub(crate) fn cursor_rect(
    cursor: (i32, i32),
    hotspot: (i32, i32),
    size: Size,
    client: Rect,
    source: Size,
    dest: Rect,
) -> Option<Rect> {
    if source.is_empty() || size.is_empty() {
        return None;
    }
    let rel_x = cursor.0 - client.x;
    let rel_y = cursor.1 - client.y;
    if rel_x < 0 || rel_y < 0 || rel_x >= client.width as i32 || rel_y >= client.height as i32 {
        return None;
    }
    let sx = f64::from(dest.width) / f64::from(source.width);
    let sy = f64::from(dest.height) / f64::from(source.height);
    let x = f64::from(dest.x) + f64::from(rel_x - hotspot.0) * sx;
    let y = f64::from(dest.y) + f64::from(rel_y - hotspot.1) * sy;
    Some(Rect::new(
        x.round() as i32,
        y.round() as i32,
        (f64::from(size.width) * sx).round().max(1.0) as u32,
        (f64::from(size.height) * sy).round().max(1.0) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_transparent_and_half_pixels() {
        // BGRA: непрозорий червоний, прозорий, напівпрозорий чорний, інверсія.
        let on_black = [0, 0, 200, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 0];
        let on_white = [0, 0, 200, 0, 255, 255, 255, 0, 127, 127, 127, 0, 0, 0, 0, 0];
        let out = premultiplied_from_backgrounds(&on_black, &on_white);
        assert_eq!(&out[0..4], &[0, 0, 200, 255]);
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);
        assert_eq!(&out[8..12], &[0, 0, 0, 128]);
        assert_eq!(&out[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn premultiplied_color_never_exceeds_alpha() {
        let on_black = [100, 100, 100, 0];
        let on_white = [200, 200, 200, 0];
        let out = premultiplied_from_backgrounds(&on_black, &on_white);
        assert_eq!(out[3], 155);
        assert!(out[..3].iter().all(|c| *c <= out[3]));
    }

    #[test]
    fn cursor_is_scaled_like_the_game() -> Result<(), &'static str> {
        // Вікно 1280×720 у (100, 100) → 2560×1440 з (0, 0): ×2.
        let client = Rect::new(100, 100, 1280, 720);
        let source = Size::new(1280, 720);
        let dest = Rect::new(0, 0, 2560, 1440);
        let r = cursor_rect((600, 400), (0, 0), Size::new(32, 32), client, source, dest)
            .ok_or("none")?;
        assert_eq!(r, Rect::new(1000, 600, 64, 64));
        // Гаряча точка зсуває зображення вліво-вгору (у збільшеному масштабі).
        let r = cursor_rect((600, 400), (8, 8), Size::new(32, 32), client, source, dest)
            .ok_or("none")?;
        assert_eq!(r, Rect::new(984, 584, 64, 64));
        Ok(())
    }

    #[test]
    fn cursor_respects_letterboxing() -> Result<(), &'static str> {
        // 4:3 на 16:9 — кадр зсунутий вправо смугою 320 px.
        let client = Rect::new(0, 0, 1024, 768);
        let r = cursor_rect(
            (0, 0),
            (0, 0),
            Size::new(32, 32),
            client,
            Size::new(1024, 768),
            Rect::new(320, 0, 1920, 1440),
        )
        .ok_or("none")?;
        assert_eq!((r.x, r.y), (320, 0));
        Ok(())
    }

    #[test]
    fn cursor_outside_game_is_not_drawn() {
        let client = Rect::new(100, 100, 1280, 720);
        let s = Size::new(1280, 720);
        let d = Rect::new(0, 0, 2560, 1440);
        assert!(cursor_rect((50, 400), (0, 0), Size::new(32, 32), client, s, d).is_none());
        assert!(cursor_rect((1380, 400), (0, 0), Size::new(32, 32), client, s, d).is_none());
        assert!(cursor_rect((600, 400), (0, 0), Size::new(0, 32), client, s, d).is_none());
    }
}
