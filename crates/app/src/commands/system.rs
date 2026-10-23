//! Службові команди: версія, каталог перекладів, налаштування, звіт про старт.

use crate::dto::{AppInfo, StartupReport};
use crate::error::CmdResult;
use crate::settings::Settings;
use crate::state::{lock, AppState};
use optimax_core::Catalog;
use std::collections::HashMap;
use tauri::State;

/// Версія, права адміністратора, версія PresentMon.
#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        elevated: optimax_core::process::is_elevated(),
        presentmon_version: optimax_frametime::presentmon::PRESENTMON_VERSION.into(),
    }
}

/// Український каталог: інтерфейс рендерить тексти `{key, args}` сам.
#[tauri::command]
pub fn get_catalog() -> CmdResult<HashMap<String, String>> {
    Ok(Catalog::ukrainian()?.entries().clone())
}

/// Що було відновлено при старті.
#[tauri::command]
pub fn startup_report(state: State<'_, AppState>) -> StartupReport {
    lock(&state.startup).clone()
}

/// Налаштування.
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    lock(&state.settings).clone()
}

/// Зберігає налаштування.
#[tauri::command]
pub fn set_settings(state: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    settings.save()?;
    if let Some(c) = lock(&state.upscale).as_ref() {
        c.apply_settings(settings.upscale, settings.upscale_hotkey);
    }
    *lock(&state.settings) = settings;
    Ok(())
}
