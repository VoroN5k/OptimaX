//! Спільні типи OptimaX: помилки, рівні впливу, знахідки, i18n, дати та доступ до реєстру.
//!
//! `unsafe` дозволено лише в модулі `registry::windows` (FFI до WinAPI).

pub mod date;
pub mod error;
pub mod finding;
pub mod i18n;
pub mod registry;

pub use date::CivilDate;
pub use error::CoreError;
pub use finding::{Category, Confidence, Evidence, Finding, Impact};
pub use i18n::{Catalog, Text};
