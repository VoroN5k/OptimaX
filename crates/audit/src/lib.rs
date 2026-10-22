//! Аудит конфігурації ігрового ПК: RAM (XMP/EXPO), частота монітора, PCIe, Resizable BAR,
//! драйвер GPU, BIOS, схема живлення, ігровий режим, HAGS.
//!
//! Структура: [`SystemProbe`] збирає дані (на Windows — [`WindowsProbe`]), чисті правила з
//! [`rules`] перетворюють їх на знахідки, [`run_audit`] збирає звіт. Аудит нічого не змінює.
//!
//! `unsafe` дозволено лише в модулі `win` (FFI до WinAPI).

pub mod devprop;
pub mod gpu_info;
pub mod model;
pub mod probe;
pub mod report;
pub mod rules;
pub mod smbios;
#[cfg(windows)]
mod win;

pub use probe::SystemProbe;
pub use report::{run_audit, AuditReport, CheckId, SkippedCheck};
#[cfg(windows)]
pub use win::WindowsProbe;
