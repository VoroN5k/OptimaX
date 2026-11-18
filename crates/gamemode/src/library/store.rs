//! Вибір людини, який переживає повторний скан: приховані ігри, визначений exe, ігри,
//! додані вручну. Файл `library.json` у теці даних програми.

use super::LibraryError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Стан однієї гри.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserGame {
    /// Підтверджене ім'я exe.
    #[serde(default)]
    pub exe: Option<String>,
    /// Прихована.
    #[serde(default)]
    pub hidden: bool,
}

/// Гра, додана вручну.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualGame {
    /// `manual:<exe в нижньому регістрі>`.
    pub id: String,
    /// Назва.
    pub name: String,
    /// Ім'я exe.
    pub exe: String,
}

/// Стан бібліотеки.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryStore {
    /// За `id` гри.
    #[serde(default)]
    pub games: BTreeMap<String, UserGame>,
    /// Додані вручну.
    #[serde(default)]
    pub manual: Vec<ManualGame>,
}

impl LibraryStore {
    /// Читає файл. Немає файлу — порожній стан; битий файл — порожній стан і попередження в лог.
    pub fn load(path: &Path) -> LibraryStore {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LibraryStore::default(),
            Err(e) => {
                tracing::warn!(
                    "бібліотека ігор: не вдалося прочитати {}: {e}",
                    path.display()
                );
                return LibraryStore::default();
            }
        };
        serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("бібліотека ігор: пошкоджений {}: {e}", path.display());
            LibraryStore::default()
        })
    }

    /// Записує файл (через тимчасовий файл і перейменування).
    pub fn save(&self, path: &Path) -> Result<(), LibraryError> {
        let io = |e: std::io::Error| LibraryError::Io(format!("{}: {e}", path.display()));
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| LibraryError::Io(format!("серіалізація: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    /// Приховує або показує гру.
    pub fn set_hidden(&mut self, id: &str, hidden: bool) {
        self.games.entry(id.to_string()).or_default().hidden = hidden;
    }

    /// Запам'ятовує або забуває exe гри.
    pub fn set_exe(&mut self, id: &str, exe: Option<String>) {
        self.games.entry(id.to_string()).or_default().exe = exe;
    }

    /// Додає гру вручну (той самий exe — оновлює назву). Повертає її `id`.
    pub fn add_manual(&mut self, name: &str, exe: &str) -> String {
        let id = format!("manual:{}", exe.to_lowercase());
        match self.manual.iter_mut().find(|m| m.id == id) {
            Some(m) => {
                m.name = name.to_string();
                m.exe = exe.to_string();
            }
            None => self.manual.push(ManualGame {
                id: id.clone(),
                name: name.to_string(),
                exe: exe.to_string(),
            }),
        }
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("optimax-libstore-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("library.json")
    }

    #[test]
    fn store_roundtrip() {
        let path = tmp("roundtrip");
        let mut s = LibraryStore::default();
        s.set_exe("epic:Rdr", Some("RDR2.exe".into()));
        s.set_hidden("steam:431960", true);
        let id = s.add_manual("Моя гра", "MyGame.exe");
        assert_eq!(id, "manual:mygame.exe");
        s.save(&path).unwrap();
        let back = LibraryStore::load(&path);
        assert_eq!(back, s);
        assert_eq!(back.games["epic:Rdr"].exe.as_deref(), Some("RDR2.exe"));
        assert!(back.games["steam:431960"].hidden);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn store_missing_file_is_empty() {
        assert_eq!(
            LibraryStore::load(Path::new(r"Z:\no\such\library.json")),
            LibraryStore::default()
        );
    }

    #[test]
    fn store_broken_file_is_empty() {
        let path = tmp("broken");
        std::fs::write(&path, "{ не json").unwrap();
        assert_eq!(LibraryStore::load(&path), LibraryStore::default());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn set_exe_none_keeps_hidden() {
        let mut s = LibraryStore::default();
        s.set_hidden("a", true);
        s.set_exe("a", Some("x.exe".into()));
        s.set_exe("a", None);
        assert_eq!(
            s.games["a"],
            UserGame {
                exe: None,
                hidden: true
            }
        );
    }

    #[test]
    fn add_manual_same_exe_updates_name() {
        let mut s = LibraryStore::default();
        s.add_manual("A", "Game.exe");
        s.add_manual("B", "GAME.exe");
        assert_eq!(s.manual.len(), 1);
        assert_eq!(s.manual[0].name, "B");
    }
}
