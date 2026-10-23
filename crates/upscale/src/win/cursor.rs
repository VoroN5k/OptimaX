//! Курсор під час апскейлу.
//!
//! Справжній курсор лишається над маленьким вікном гри (там, куди мають іти кліки), а в
//! оверлеї видно збільшений курсор із кадру захоплення. Тому, поки оверлей видно:
//! - курсор обмежено клієнтською областю гри (`ClipCursor`), щоб клік не пішов повз гру;
//! - системний курсор сховано (`MagShowSystemCursor`), щоб не було двох курсорів.
//!
//! Усе повертається, щойно оверлей ховається (Alt+Tab) або апскейл зупиняється.

use crate::geometry::Rect;
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::Magnification::{MagInitialize, MagShowSystemCursor, MagUninitialize};
use windows::Win32::UI::WindowsAndMessaging::{ClipCursor, GetClipCursor};

pub(super) struct CursorControl {
    /// Magnification API ініціалізовано (без нього лише обмежуємо, не ховаємо).
    mag: bool,
    hidden: bool,
    /// Прямокутник, яким ми обмежили курсор.
    clip: Option<RECT>,
}

fn to_rect(r: Rect) -> RECT {
    RECT {
        left: r.x,
        top: r.y,
        right: r
            .x
            .saturating_add(i32::try_from(r.width).unwrap_or(i32::MAX)),
        bottom: r
            .y
            .saturating_add(i32::try_from(r.height).unwrap_or(i32::MAX)),
    }
}

fn contains(outer: &RECT, inner: &RECT) -> bool {
    inner.left >= outer.left
        && inner.top >= outer.top
        && inner.right <= outer.right
        && inner.bottom <= outer.bottom
}

fn current_clip() -> Option<RECT> {
    let mut r = RECT::default();
    // SAFETY: локальний RECT.
    unsafe { GetClipCursor(&mut r) }.ok().map(|()| r)
}

impl CursorControl {
    pub fn new() -> Self {
        // SAFETY: без аргументів; парний MagUninitialize у Drop.
        let mag = unsafe { MagInitialize() }.as_bool();
        if mag {
            // Якщо попередній сеанс завершився аварійно, курсор міг лишитися схованим.
            // SAFETY: Magnification API ініціалізовано вище.
            let _ = unsafe { MagShowSystemCursor(true) };
        }
        Self {
            mag,
            hidden: false,
            clip: None,
        }
    }

    /// `active` — оверлей видно; `client` — клієнтська область гри; `hide` — ховати системний
    /// курсор (лише коли збільшений курсор малюється з кадру захоплення).
    pub fn update(&mut self, active: bool, client: Rect, hide: bool) {
        if !active {
            self.release();
            return;
        }
        let target = to_rect(client);
        // Гра могла обмежити курсор сама (тісніше, напр. у стрілялці) — тоді не чіпаємо.
        let keep = current_clip().is_some_and(|c| contains(&target, &c));
        if !keep {
            // SAFETY: локальний RECT у координатах робочого столу.
            if unsafe { ClipCursor(Some(&target)) }.is_ok() {
                self.clip = Some(target);
            }
        }
        if hide && self.mag && !self.hidden {
            // SAFETY: Magnification API ініціалізовано.
            self.hidden = unsafe { MagShowSystemCursor(false) }.as_bool();
        } else if !hide && self.hidden {
            self.show();
        }
    }

    fn show(&mut self) {
        if self.hidden {
            // SAFETY: Magnification API ініціалізовано (інакше `hidden` не став би true).
            let _ = unsafe { MagShowSystemCursor(true) };
            self.hidden = false;
        }
    }

    /// Знімає наші обмеження. Чужий (ігровий) `ClipCursor` не скидаємо.
    pub fn release(&mut self) {
        self.show();
        if let Some(ours) = self.clip.take() {
            if current_clip().is_some_and(|c| c == ours) {
                // SAFETY: None знімає обмеження.
                let _ = unsafe { ClipCursor(None) };
            }
        }
    }
}

impl Drop for CursorControl {
    fn drop(&mut self) {
        self.release();
        if self.mag {
            // SAFETY: парний до успішного MagInitialize у цьому самому потоці.
            let _ = unsafe { MagUninitialize() };
        }
    }
}
