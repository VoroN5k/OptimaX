//! Команди екрана «Ноутбук»: стан батареї та режим живлення.

use super::blocking;
use crate::error::CmdResult;
use crate::state::POWER;
use optimax_audit::{SystemProbe, WindowsProbe};
use optimax_core::power::PowerApi;
use optimax_audit::model::BatteryInfo;
use optimax_core::finding::Finding;
use optimax_audit::rules::check_battery;
use serde::Serialize;

/// Стан ноутбука для UI.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LaptopView {
    /// Чи є батарея (false — десктоп або батарею не виявлено).
    pub is_laptop: bool,
    /// Дані батареї; `None` якщо немає.
    pub battery: Option<BatteryInfo>,
    /// Чи доступний недокументований API оверлейного режиму.
    pub overlay_supported: bool,
    /// GUID поточного оверлейного режиму; `None` = збалансований.
    pub overlay_guid: Option<String>,
    /// Знахідки аудиту батареї.
    pub findings: Vec<Finding>,
}

/// Збирає дані ноутбука.
#[tauri::command]
pub async fn laptop_info() -> CmdResult<LaptopView> {
    blocking(|| {
        let probe = WindowsProbe::new();
        let battery = probe.battery_info()?;
        let is_laptop = battery.is_some();

        let findings = battery
            .as_ref()
            .map(|b| check_battery(b))
            .unwrap_or_default();

        let overlay_supported = POWER.overlay_supported();
        let overlay_guid = if overlay_supported {
            POWER.overlay_scheme().unwrap_or(None)
        } else {
            None
        };

        Ok(LaptopView {
            is_laptop,
            battery,
            overlay_supported,
            overlay_guid,
            findings,
        })
    })
    .await
}
