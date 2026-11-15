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
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // Тимчасова тека може мати коротке ім'я 8.3 (`C:\Users\RUNNER~1\…` на GitHub Actions);
        // `safefs` свідомо відхиляє такий шлях як перенаправлення — беремо довгий.
        let base = std::fs::canonicalize(&base).unwrap();
        let (old, new) = (base.join("old"), base.join("new"));
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

// ---- Дані інтерфейсу між захищеною текою й текою користувача ----

/// Файли даних інтерфейсу в корені теки.
const UI_FILES: [&str; 2] = ["settings.json", "last_game.json"];
/// Теки даних інтерфейсу (файли в них — без вкладених тек).
const UI_DIRS: [&str; 4] = ["history", "history/sessions", "stability", "experiments"];
/// Найбільший файл, який переносимо (стиснений запис гри на 10 хв — до десятків МБ).
const UI_LIMIT: u64 = 128 * 1024 * 1024;

/// Чи схоже на наш файл: JSON або стиснений JSON (`.gz`). З теки користувача в захищену
/// переноситься лише таке.
fn looks_like_ours(name: &str, bytes: &[u8]) -> bool {
    if name.ends_with(".gz") {
        let mut out = Vec::new();
        let ok = std::io::Read::read_to_end(
            &mut std::io::Read::take(flate2::read::GzDecoder::new(bytes), UI_LIMIT * 4),
            &mut out,
        )
        .is_ok();
        ok && serde_json::from_slice::<serde_json::Value>(&out).is_ok()
    } else {
        serde_json::from_slice::<serde_json::Value>(bytes).is_ok()
    }
}

/// Мітка часу зміни, мс (для «новіший перемагає» у налаштуваннях).
fn modified_ms(p: &Path) -> Option<u128> {
    std::fs::metadata(p)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis())
}

/// Переносить відсутні в `to` файли з `from`; налаштування й останню гру — якщо в `from` новіші.
/// `untrusted` — `from` у руках користувача: читаємо лише безпечно й перевіряємо формат.
/// Пише лише в `to` (процес з правами адміністратора — лише в захищену теку).
pub(crate) fn sync_dirs(from: &Path, to: &Path, untrusted: bool) -> Vec<String> {
    let mut moved = Vec::new();
    let mut copy = |rel: &str, newer_wins: bool| {
        // Шлях — з окремих частин: `safefs` звіряє справжній шлях, а `/` у ньому не збігся б.
        let join = |base: &Path| rel.split('/').fold(base.to_path_buf(), |p, c| p.join(c));
        let (src, dst) = (join(from), join(to));
        let name = rel.rsplit('/').next().unwrap_or(rel);
        // Позначка незавершеного тесту стабільності належить своєму режиму.
        if name.starts_with("running.") {
            return;
        }
        if dst.exists() && !(newer_wins && modified_ms(&src) > modified_ms(&dst)) {
            return;
        }
        let bytes = match safefs::read_limited(&src, UI_LIMIT) {
            Ok(Some(b)) => b,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("{}: не перенесено: {e}", src.display());
                return;
            }
        };
        if untrusted && !looks_like_ours(name, &bytes) {
            tracing::warn!("{}: не перенесено — незнайомий формат", src.display());
            return;
        }
        if let Some(parent) = dst.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        if std::fs::write(&dst, &bytes).is_ok() {
            moved.push(rel.to_string());
        }
    };
    for f in UI_FILES {
        copy(f, true);
    }
    for d in UI_DIRS {
        let dir = d.split('/').fold(from.to_path_buf(), |p, c| p.join(c));
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in entries.filter_map(Result::ok) {
            // Лише звичайні файли; посилання й теки пропускаємо.
            if e.file_type().is_ok_and(|t| t.is_file()) {
                if let Some(n) = e.file_name().to_str() {
                    copy(&format!("{d}/{n}"), false);
                }
            }
        }
    }
    moved
}

/// Вирівнює дані інтерфейсу між режимами: без прав адміністратора — з захищеної теки (читати її
/// користувач може) у теку користувача; з правами — навпаки, з перевіркою формату.
pub fn sync_ui_data() {
    let elevated = optimax_core::process::is_elevated();
    let (Some(user), Some(protected)) = (
        optimax_core::datadir::user_dir(),
        optimax_core::datadir::protected_path(),
    ) else {
        return;
    };
    let (from, to, untrusted) = if elevated {
        (user, protected, true)
    } else {
        (protected, user, false)
    };
    if !from.is_dir() {
        return;
    }
    let moved = sync_dirs(&from, &to, untrusted);
    if !moved.is_empty() {
        tracing::info!(
            "дані інтерфейсу з {} у {}: {} файлів",
            from.display(),
            to.display(),
            moved.len()
        );
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;

    /// Тимчасова тека з повним (довгим) шляхом: на GitHub Actions `TEMP` — з коротким іменем 8.3
    /// (`C:\Users\RUNNER~1\…`), а `safefs` такий шлях свідомо відхиляє як «перенаправлений».
    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("optimax-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("тека");
        let full = std::fs::canonicalize(&d).expect("справжній шлях");
        let full = full.to_str().expect("UTF-8");
        std::path::PathBuf::from(full.strip_prefix(r"\\?\").unwrap_or(full))
    }

    #[test]
    fn missing_files_move_and_untrusted_junk_does_not() {
        let (from, to) = (tmp("from"), tmp("to"));
        std::fs::create_dir_all(from.join("history")).expect("history");
        std::fs::create_dir_all(from.join("stability")).expect("stability");
        std::fs::write(from.join("settings.json"), br#"{"economy":true}"#).expect("s");
        std::fs::write(from.join("history/1-a.json"), br#"{"id":"1"}"#).expect("h");
        std::fs::write(from.join("history/2-b.json"), b"MZ\x90\x00 not json").expect("junk");
        std::fs::write(from.join("stability/running.json"), b"{}").expect("marker");
        std::fs::create_dir_all(to.join("history")).expect("to/history");
        std::fs::write(to.join("history/1-a.json"), br#"{"id":"mine"}"#).expect("наявний");
        let mut moved = sync_dirs(&from, &to, true);
        moved.sort();
        assert_eq!(moved, ["settings.json"]);
        // Наявний файл не перезаписується; сміття й позначка тесту не переносяться.
        assert_eq!(
            std::fs::read(to.join("history/1-a.json")).expect("r"),
            br#"{"id":"mine"}"#
        );
        assert!(!to.join("history/2-b.json").exists());
        assert!(!to.join("stability/running.json").exists());
        // З довіреної теки формат не перевіряється.
        let moved = sync_dirs(&from, &to, false);
        assert_eq!(moved, ["history/2-b.json"]);
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }
}
