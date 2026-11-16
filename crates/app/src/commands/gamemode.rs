//! Режим гри: профілі та стежник (подія `gamemode`). Сама логіка — у `crate::session`. Якщо
//! працює агент у треї, режим гри веде він (рішення користувача 2026-11-16): ці команди лише
//! кажуть йому, що робити (`crate::agent_link`), а стан приходить тими самими подіями.

use crate::agent_link;
use crate::dto::ProfilePreview;
use crate::error::{CmdError, CmdResult};
use crate::profiles;
use crate::session::gamemode::Watcher;
use crate::state::{lock, AppState};
use optimax_core::Text;
use optimax_gamemode::GameProfile;
use tauri::{AppHandle, State};

/// Профілі ігор.
#[tauri::command]
pub fn profiles_list(state: State<'_, AppState>) -> CmdResult<Vec<GameProfile>> {
    profiles::load(&state.link)
}

/// Безпечна версія профілю й попередження — для показу перед збереженням.
#[tauri::command]
pub fn profile_preview(profile: GameProfile) -> CmdResult<ProfilePreview> {
    let (profile, warnings) = profile.sanitized()?;
    Ok(ProfilePreview { profile, warnings })
}

/// Зберігає профіль (безпечну версію). Повертає попередження.
#[tauri::command]
pub fn profile_save(state: State<'_, AppState>, profile: GameProfile) -> CmdResult<Vec<Text>> {
    profiles::upsert(&state.link, &profile)
}

/// Видаляє профіль.
#[tauri::command]
pub fn profile_remove(state: State<'_, AppState>, exe: String) -> CmdResult<bool> {
    profiles::remove(&state.link, &exe)
}

fn agent_config(f: impl FnOnce(&mut agent_link::AgentConfig)) -> CmdResult<()> {
    agent_link::update_config(f)
        .map_err(|e| CmdError::Failed(format!("агент у треї: налаштування не записано: {e}")))
}

/// Чи працює стежник (у вікні чи в агенті).
#[tauri::command]
pub fn gamemode_running(state: State<'_, AppState>) -> bool {
    if agent_link::agent_active() {
        return agent_link::read_config().gamemode;
    }
    lock(&state.gamemode).is_some()
}

/// Дозволяє режиму гри працювати без точки відновлення (до кінця цього запуску програми чи
/// агента).
#[tauri::command]
pub fn gamemode_allow_without_restore_point(state: State<'_, AppState>) -> CmdResult<()> {
    *lock(&state.session.allow_without_restore) = true;
    if agent_link::agent_active() {
        agent_config(|c| c.allow_without_restore = true)?;
    }
    Ok(())
}

/// Запускає стежник за профілями (або просить агента).
#[tauri::command]
pub fn gamemode_start(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let profiles = profiles::load(&state.link)?;
    if profiles.is_empty() {
        return Err(CmdError::Failed(
            "немає профілів ігор — спершу створи профіль".into(),
        ));
    }
    if agent_link::agent_active() {
        return agent_config(|c| c.gamemode = true);
    }
    let mut slot = lock(&state.gamemode);
    if slot.is_some() {
        return Ok(());
    }
    *slot = Some(Watcher::start(super::session::host(&app), profiles)?);
    Ok(())
}

/// Зупиняє стежник; якщо гра йде — усе повертається.
#[tauri::command]
pub fn gamemode_stop(state: State<'_, AppState>) -> CmdResult<()> {
    if agent_link::agent_active() {
        agent_config(|c| c.gamemode = false)?;
    }
    let watcher = lock(&state.gamemode).take();
    if let Some(w) = watcher {
        w.shutdown();
    }
    Ok(())
}
