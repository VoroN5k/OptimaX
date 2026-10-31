//! Одноразове перенесення даних із колишньої теки `%LOCALAPPDATA%\OptimaX` у захищену
//! (`%ProgramData%\OptimaX\<SID>`, див. [`optimax_core::datadir`]).
//!
//! Стара тека доступна на запис будь-якому процесу користувача, тож її вмісту не довіряємо:
//! кожен файл читається без розкриття посилань і з обмеженням розміру, переноситься лише якщо
//! розбирається як очікуваний тип, і записується заново з цього типу. Записи журналу змін і
//! карантину додатково перевіряються під час використання (відкат, відновлення). Старі файли
//! не видаляються. Файли, що вже є в новій теці, не перезаписуються.
//!
//! Файли карантину не переносяться: партії в індексі й далі вказують на стару теку, звідки їх
//! можна відновити чи видалити, доки не мине термін.

use optimax_cleanup::{JournalRecord, QuarantineBatch, Snapshot};
use optimax_core::safefs;
use optimax_gamemode::GameProfile;
use optimax_tweaks::JournalEntry;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

/// Мітка: перенесення вже було (успішне чи ні).
const MARKER: &str = "migrated-from-localappdata.json";
/// Найбільший файл, який читаємо зі старої теки.
const LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Serialize)]
struct Marker {
    at_unix: u64,
    from: String,
    moved: Vec<String>,
    skipped: Vec<(String, String)>,
}

/// Переносить один файл певного типу.
type Mover = fn(&Path, &Path, &str) -> Result<bool, String>;

/// Читає `old/name`, перевіряє тип `T` і пише в `new/name`. `Ok(false)` — файлу немає або
/// в новій теці вже є свій.
fn move_typed<T: DeserializeOwned + Serialize>(
    old: &Path,
    new: &Path,
    name: &str,
) -> Result<bool, String> {
    let target = new.join(name);
    if target.exists() {
        return Ok(false);
    }
    let Some(bytes) = safefs::read_limited(&old.join(name), LIMIT).map_err(|e| e.to_string())?
    else {
        return Ok(false);
    };
    let value: T = serde_json::from_slice(&bytes).map_err(|e| format!("не той формат: {e}"))?;
    let json = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    std::fs::write(&target, json).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Переносить дані, якщо цього ще не робили. Помилки окремих файлів не зупиняють решту.
pub fn migrate_legacy() {
    let (Ok(new), Some(old)) = (
        optimax_core::datadir::data_dir(),
        optimax_core::datadir::legacy_dir(),
    ) else {
        return;
    };
    if new.join(MARKER).exists() {
        return;
    }
    let mut marker = Marker {
        at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        from: old.display().to_string(),
        moved: Vec::new(),
        skipped: Vec::new(),
    };
    if old.is_dir() {
        let mut files: Vec<(String, Mover)> = vec![
            (
                "settings.json".into(),
                move_typed::<crate::settings::Settings>,
            ),
            ("journal.json".into(), move_typed::<Vec<JournalEntry>>),
            ("profiles.json".into(), move_typed::<Vec<GameProfile>>),
            ("quarantine.json".into(), move_typed::<Vec<QuarantineBatch>>),
            (
                "cleanup-journal.json".into(),
                move_typed::<Vec<JournalRecord>>,
            ),
        ];
        for letter in 'A'..='Z' {
            files.push((
                format!("snapshots-{letter}.json"),
                move_typed::<Vec<Snapshot>>,
            ));
        }
        for (name, mover) in files {
            match mover(&old, new, &name) {
                Ok(true) => marker.moved.push(name),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!("перенесення {name} зі старої теки пропущено: {e}");
                    marker.skipped.push((name, e));
                }
            }
        }
        tracing::info!(
            "дані перенесено з {} у {}: {:?}; стара тека більше не використовується",
            old.display(),
            new.display(),
            marker.moved
        );
    }
    let written = serde_json::to_string_pretty(&marker)
        .map_err(std::io::Error::other)
        .and_then(|j| std::fs::write(new.join(MARKER), j));
    if let Err(e) = written {
        tracing::warn!("не вдалося записати мітку перенесення даних: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_well_formed_files_move_and_nothing_is_overwritten() {
        let base = std::env::temp_dir().join(format!("optimax-migrate-{}", std::process::id()));
        let (old, new) = (base.join("old"), base.join("new"));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("journal.json"), "[]").unwrap();
        std::fs::write(old.join("profiles.json"), r#"{"not":"a list"}"#).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        std::fs::write(new.join("settings.json"), "mine").unwrap();

        assert_eq!(
            move_typed::<Vec<JournalEntry>>(&old, &new, "journal.json"),
            Ok(true)
        );
        assert!(move_typed::<Vec<GameProfile>>(&old, &new, "profiles.json").is_err());
        assert!(!new.join("profiles.json").exists());
        assert_eq!(
            move_typed::<crate::settings::Settings>(&old, &new, "settings.json"),
            Ok(false)
        );
        assert_eq!(
            std::fs::read_to_string(new.join("settings.json")).unwrap(),
            "mine"
        );
        assert_eq!(
            move_typed::<Vec<QuarantineBatch>>(&old, &new, "quarantine.json"),
            Ok(false),
            "файлу немає — нічого не робимо"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
