//! Вікна, які можна масштабувати.

use crate::geometry::{Rect, Size};
use crate::imp;

/// Ідентифікатор вікна верхнього рівня (HWND). Дійсний, поки вікно існує.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub(crate) isize);

/// Відомості про вікно.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// Ідентифікатор.
    pub id: WindowId,
    /// PID процесу-власника.
    pub pid: u32,
    /// Ім'я exe процесу (напр. `GeometryDash.exe`); порожнє, якщо визначити не вдалося.
    pub exe: String,
    /// Заголовок.
    pub title: String,
    /// Розмір клієнтської області (без рамки й заголовка), фізичні пікселі.
    pub client: Size,
    /// Монітор, на якому вікно (фізичні пікселі, координати робочого столу).
    pub monitor: Rect,
    /// Чи згорнуте.
    pub minimized: bool,
}

/// Вікно на передньому плані (його корінь, якщо в фокусі дочірнє).
pub fn foreground_window() -> Option<WindowInfo> {
    imp::foreground_window()
}

/// Відомості про вікно `id` (`None`, якщо його вже немає).
pub fn window_info(id: WindowId) -> Option<WindowInfo> {
    imp::window_info(id)
}

/// Головне вікно процесу `pid`: найбільше видиме вікно верхнього рівня без власника.
pub fn main_window_of(pid: u32) -> Option<WindowInfo> {
    imp::main_window_of(pid)
}

/// Чи підтримує система Windows Graphics Capture (Windows 10 1903+).
pub fn is_supported() -> bool {
    imp::is_supported()
}

/// Вмикає Per-Monitor DPI v2 для процесу, щоб координати вікон були у фізичних пікселях.
/// Застосунок OptimaX робить це маніфестом; функція — для консольних прикладів.
pub fn enable_per_monitor_dpi() {
    imp::enable_per_monitor_dpi();
}
