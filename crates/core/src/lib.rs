//! Спільні типи OptimaX: помилки, рівні впливу, знахідки, i18n, дати та доступ до реєстру.
//!
//! `unsafe` дозволено лише в платформних модулях `*::windows`, `clock`, `dialog`, `input` і `toast` (FFI до WinAPI).

pub mod clock;
pub mod datadir;
pub mod date;
pub mod dialog;
pub mod error;
pub mod finding;
pub mod i18n;
pub mod input;
pub mod memory;
pub mod power;
pub mod process;
pub mod registry;
pub mod safefs;
pub mod service;
pub mod toast;

pub use date::CivilDate;
pub use error::CoreError;
pub use finding::{Category, Confidence, Evidence, Finding, FindingBuilder, Impact};
pub use i18n::{Catalog, Text};
