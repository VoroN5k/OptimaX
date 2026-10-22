//! Спільні типи OptimaX: помилки, рівні впливу, знахідки, i18n, дати та доступ до реєстру.
//!
//! `unsafe` дозволено лише в платформних модулях `*::windows` і `clock` (FFI до WinAPI).

pub mod clock;
pub mod date;
pub mod error;
pub mod finding;
pub mod i18n;
pub mod memory;
pub mod power;
pub mod process;
pub mod registry;
pub mod service;

pub use date::CivilDate;
pub use error::CoreError;
pub use finding::{Category, Confidence, Evidence, Finding, FindingBuilder, Impact};
pub use i18n::{Catalog, Text};
