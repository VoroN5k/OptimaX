//! Журнал змін: що, де, старе й нове значення, коли. Записи не видаляються — це історія,
//! за якою будь-яку зміну можна відкотити. Зберігається атомарно (тимчасовий файл + rename).

use crate::change::{SettingTarget, SettingValue};
use crate::restore::RestorePointStatus;
use crate::TweakError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// Стан запису.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EntryState {
    /// Записано перед зміною; якщо програма впала — зміна могла не завершитись.
    Pending,
    /// Зміну застосовано й перевірено.
    Applied,
    /// Разову дію виконано (відкату не потребує, напр. очищення standby).
    Performed,
    /// Зміну відкочено.
    RolledBack {
        /// Unix-час відкату.
        at_unix: i64,
    },
    /// Зміна не вдалася (і її наслідки відкочено).
    Failed {
        /// Причина.
        error: String,
    },
}

/// Запис журналу.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JournalEntry {
    /// Номер (зростає).
    pub id: u64,
    /// Твік.
    pub tweak_id: String,
    /// Де.
    pub target: SettingTarget,
    /// Значення до зміни.
    pub old: SettingValue,
    /// Значення після зміни.
    pub new: SettingValue,
    /// Unix-час створення запису.
    pub created_unix: i64,
    /// Стан.
    pub state: EntryState,
    /// Точка відновлення, створена в цій сесії перед першою зміною.
    pub restore_point: Option<RestorePointStatus>,
}

/// Сховище журналу.
pub trait JournalStore: Send + Sync {
    /// Завантажує всі записи (порожньо, якщо журналу ще немає).
    fn load(&self) -> Result<Vec<JournalEntry>, TweakError>;
    /// Зберігає всі записи.
    fn save(&self, entries: &[JournalEntry]) -> Result<(), TweakError>;
}

/// Журнал у JSON-файлі.
#[derive(Debug, Clone)]
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    /// Журнал за шляхом.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `%LOCALAPPDATA%\OptimaX\journal.json`.
    pub fn default_location() -> Option<Self> {
        std::env::var_os("LOCALAPPDATA")
            .map(|d| Self::new(PathBuf::from(d).join("OptimaX").join("journal.json")))
    }

    /// Шлях до файлу.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn io_err(e: impl std::fmt::Display) -> TweakError {
    TweakError::Journal(e.to_string())
}

impl JournalStore for FileStore {
    fn load(&self) -> Result<Vec<JournalEntry>, TweakError> {
        match fs::read_to_string(&self.path) {
            Ok(s) if s.trim().is_empty() => Ok(Vec::new()),
            Ok(s) => serde_json::from_str(&s).map_err(io_err),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(io_err(e)),
        }
    }

    fn save(&self, entries: &[JournalEntry]) -> Result<(), TweakError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(io_err)?;
        }
        let json = serde_json::to_string_pretty(entries).map_err(io_err)?;
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp).map_err(io_err)?;
            f.write_all(json.as_bytes()).map_err(io_err)?;
            f.sync_all().map_err(io_err)?;
        }
        // rename на Windows замінює файл атомарно (MoveFileEx з REPLACE_EXISTING).
        fs::rename(&tmp, &self.path).map_err(io_err)
    }
}

/// Журнал у пам'яті (тести).
#[derive(Debug, Default)]
pub struct MemStore {
    entries: Mutex<Vec<JournalEntry>>,
}

impl MemStore {
    /// Порожній журнал.
    pub fn new() -> Self {
        Self::default()
    }

    /// Копія записів.
    pub fn snapshot(&self) -> Vec<JournalEntry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl JournalStore for MemStore {
    fn load(&self) -> Result<Vec<JournalEntry>, TweakError> {
        Ok(self.snapshot())
    }

    fn save(&self, entries: &[JournalEntry]) -> Result<(), TweakError> {
        *self.entries.lock().unwrap_or_else(PoisonError::into_inner) = entries.to_vec();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::registry::{RegHive, RegValue};

    fn entry(id: u64) -> JournalEntry {
        JournalEntry {
            id,
            tweak_id: "game_mode".into(),
            target: SettingTarget::Registry {
                hive: RegHive::CurrentUser,
                path: r"Software\Microsoft\GameBar".into(),
                name: "AutoGameModeEnabled".into(),
            },
            old: SettingValue::Registry(None),
            new: SettingValue::Registry(Some(RegValue::Dword(0))),
            created_unix: 1_700_000_000,
            state: EntryState::Applied,
            restore_point: Some(RestorePointStatus::Created { sequence: 42 }),
        }
    }

    #[test]
    fn file_store_roundtrip_and_missing_file() {
        let dir = std::env::temp_dir().join(format!("optimax-journal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = FileStore::new(dir.join("journal.json"));
        assert!(store.load().unwrap().is_empty());
        let entries = vec![entry(1), entry(2)];
        store.save(&entries).unwrap();
        assert_eq!(store.load().unwrap(), entries);
        // Тип значення зберігається явно (вимога: ключ, ім'я, тип, старе значення, час).
        let raw = fs::read_to_string(store.path()).unwrap();
        assert!(raw.contains("\"Dword\""));
        assert!(raw.contains("AutoGameModeEnabled"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_journal_is_an_error_not_silently_empty() {
        let dir = std::env::temp_dir().join(format!("optimax-journal-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = FileStore::new(dir.join("journal.json"));
        fs::write(store.path(), "{ not json").unwrap();
        assert!(store.load().is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
