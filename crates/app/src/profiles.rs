//! Профілі ігор: через службу, якщо вона доступна (захищену теку пише лише процес із правами
//! адміністратора), інакше — файлом у цьому процесі. Логіка однакова: `ProfileStore`.

use crate::error::{CmdError, CmdResult};
use crate::link::Link;
use optimax_core::Text;
use optimax_gamemode::{GameProfile, ProfileStore};
use optimax_service::protocol::Command;

fn store() -> CmdResult<ProfileStore> {
    ProfileStore::default_location().map_err(|e| CmdError::Failed(e.to_string()))
}

/// Усі профілі.
pub fn load(link: &Link) -> CmdResult<Vec<GameProfile>> {
    match link.ask_service(Command::ProfilesList) {
        Some(r) => r,
        None => Ok(store()?.load()?),
    }
}

/// Профіль за ім'ям exe.
pub fn find(link: &Link, exe: &str) -> CmdResult<Option<GameProfile>> {
    Ok(load(link)?
        .into_iter()
        .find(|p| p.exe.eq_ignore_ascii_case(exe)))
}

/// Додає або замінює профіль (зберігається безпечна версія). Повертає попередження.
pub fn upsert(link: &Link, profile: &GameProfile) -> CmdResult<Vec<Text>> {
    let cmd = Command::ProfileSave {
        profile: serde_json::to_value(profile)?,
    };
    match link.ask_service(cmd) {
        Some(r) => r,
        None => Ok(store()?.upsert(profile)?),
    }
}

/// Видаляє профіль. `false`, якщо такого не було.
pub fn remove(link: &Link, exe: &str) -> CmdResult<bool> {
    match link.ask_service(Command::ProfileRemove { exe: exe.into() }) {
        Some(r) => r,
        None => Ok(store()?.remove(exe)?),
    }
}
