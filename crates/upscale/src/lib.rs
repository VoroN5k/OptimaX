//! Апскейл вікна гри для OptimaX.
//!
//! Гра працює у вікні або borderless на нижчій роздільності; OptimaX захоплює вікно через
//! Windows Graphics Capture і малює збільшений кадр у власному вікні поверх гри на весь
//! монітор. Оверлей не забирає фокус і пропускає мишу, тож ввід іде в гру.
//! Ігри в ексклюзивному повноекранному режимі захопити неможливо — це обмеження Windows.
//!
//! - [`geometry`]: що вирізати з кадру й куди малювати (чисті функції, тести).
//! - [`settings`]: фільтр (точковий, білінійний, Lanczos, FSR 1, NIS), різкість, вписування, курсор.
//! - [`Upscaler`]: запущений сеанс в окремому потоці.
//! - [`toggle::HotkeyToggle`]: увімкнення / вимкнення за `Ctrl+Shift+F10`.
//!
//! `unsafe` дозволено лише в платформному модулі `imp` (FFI до WinAPI, Direct3D 11, WinRT).

pub mod error;
pub mod geometry;
pub mod session;
pub mod settings;
mod shaders;
pub mod toggle;
pub mod window;

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "win/mod.rs"]
mod imp;

#[cfg(not(windows))]
#[path = "stub.rs"]
mod imp;

pub use error::UpscaleError;
pub use geometry::{FitMode, Rect, Size};
pub use session::{EndReason, UpscaleStats, Upscaler};
pub use settings::{Filter, UpscaleSettings};
pub use toggle::{HotkeyToggle, ToggleEvent};
pub use window::{
    foreground_window, is_supported, main_window_of, window_info, WindowId, WindowInfo,
};
