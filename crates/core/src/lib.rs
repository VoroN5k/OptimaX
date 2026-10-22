//! Спільні типи OptimaX: помилки, рівні впливу, знахідки, i18n, дати та доступ до реєстру.
//!
//! `unsafe` дозволено лише в модулях `registry::windows` і `clock` (FFI до WinAPI).

pub mod clock;
pub mod date;
pub mod error;
pub mod finding;
pub mod i18n;
pub mod registry;

pub use date::CivilDate;
pub use error::CoreError;
pub use finding::{Category, Confidence, Evidence, Finding, FindingBuilder, Impact};
pub use i18n::{Catalog, Text};
