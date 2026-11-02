//! Сховище історії: по файлу на запис (`<id>.json`) і, для останніх записів кожної гри, повний
//! запис (`<id>.rec.gz`, байти дає застосунок). Файлова система — за трейтом [`Files`].

use crate::model::HistoryEntry;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

/// Скільки повних записів тримати на кожну гру (старіші лишаються підсумком).
pub const KEEP_RECORDINGS_PER_GAME: usize = 10;

const ENTRY_EXT: &str = ".json";
const REC_EXT: &str = ".rec.gz";

/// Помилка історії.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// Помилка файлової системи.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Пошкоджений або несумісний файл.
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    /// Ідентифікатор не схожий на наш (захист від шляхів на кшталт `..\\`).
    #[error("недопустимий ідентифікатор запису: {0}")]
    BadId(String),
}

/// Файли в одній теці (лише імена, без шляхів).
pub trait Files {
    /// Імена файлів.
    fn list(&self) -> Result<Vec<String>, HistoryError>;
    /// Вміст (`None` — файлу немає).
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, HistoryError>;
    /// Записати (замінює наявний цілком).
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), HistoryError>;
    /// Видалити (відсутній файл — не помилка).
    fn remove(&self, name: &str) -> Result<(), HistoryError>;
}

/// Тека на диску.
pub struct StdDir {
    dir: PathBuf,
}

impl StdDir {
    /// Тека (створюється, якщо її немає).
    pub fn new(dir: PathBuf) -> Result<Self, HistoryError> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }
}

impl Files for StdDir {
    fn list(&self) -> Result<Vec<String>, HistoryError> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.dir)? {
            let e = e?;
            if e.file_type()?.is_file() {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
        Ok(out)
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, HistoryError> {
        match std::fs::read(self.dir.join(name)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), HistoryError> {
        // Спершу тимчасовий файл, потім перейменування: збій посеред запису не лишає обрізаний JSON.
        let tmp = self.dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, self.dir.join(name))?;
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), HistoryError> {
        match std::fs::remove_file(self.dir.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

/// Файли в пам'яті (для тестів).
#[derive(Default)]
pub struct MemFiles {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemFiles {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        self.files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Files for MemFiles {
    fn list(&self) -> Result<Vec<String>, HistoryError> {
        Ok(self.map().keys().cloned().collect())
    }
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, HistoryError> {
        Ok(self.map().get(name).cloned())
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), HistoryError> {
        self.map().insert(name.to_string(), bytes.to_vec());
        Ok(())
    }
    fn remove(&self, name: &str) -> Result<(), HistoryError> {
        self.map().remove(name);
        Ok(())
    }
}

/// Ідентифікатор запису: час і ім'я гри лише з безпечних символів (`1793000000000-re9.exe`).
pub fn entry_id(game: &str, at_unix_ms: i64) -> String {
    let safe: String = game
        .chars()
        .take(60)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{at_unix_ms}-{safe}")
}

/// Ідентифікатор із інтерфейсу: лише те, що могла створити [`entry_id`].
pub(crate) fn checked(id: &str) -> Result<&str, HistoryError> {
    let ok = !id.is_empty()
        && id.len() <= 100
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && !id.contains("..");
    if ok {
        Ok(id)
    } else {
        Err(HistoryError::BadId(id.to_string()))
    }
}

/// Історія записів.
pub struct Store<'a> {
    files: &'a dyn Files,
}

impl<'a> Store<'a> {
    /// Історія в цих файлах.
    pub fn new(files: &'a dyn Files) -> Self {
        Self { files }
    }

    /// Зберігає запис і (якщо є) повний запис гри.
    pub fn save(&self, entry: &HistoryEntry, recording: Option<&[u8]>) -> Result<(), HistoryError> {
        let id = checked(&entry.id)?;
        let mut e = entry.clone();
        e.has_recording = recording.is_some();
        if let Some(bytes) = recording {
            self.files.write(&format!("{id}{REC_EXT}"), bytes)?;
        }
        self.files
            .write(&format!("{id}{ENTRY_EXT}"), &serde_json::to_vec_pretty(&e)?)
    }

    /// Усі записи, найновіші першими, і скільки файлів не вдалося прочитати.
    pub fn list(&self) -> Result<(Vec<HistoryEntry>, usize), HistoryError> {
        let mut out = Vec::new();
        let mut broken = 0;
        for name in self.files.list()? {
            let Some(id) = name.strip_suffix(ENTRY_EXT) else {
                continue;
            };
            if checked(id).is_err() {
                continue;
            }
            match self
                .files
                .read(&name)?
                .map(|b| serde_json::from_slice::<HistoryEntry>(&b))
            {
                Some(Ok(e)) => out.push(e),
                Some(Err(_)) => broken += 1,
                None => {}
            }
        }
        out.sort_by(|a, b| b.at_unix_ms.cmp(&a.at_unix_ms).then(b.id.cmp(&a.id)));
        Ok((out, broken))
    }

    /// Один запис.
    pub fn get(&self, id: &str) -> Result<Option<HistoryEntry>, HistoryError> {
        let id = checked(id)?;
        match self.files.read(&format!("{id}{ENTRY_EXT}"))? {
            Some(b) => Ok(Some(serde_json::from_slice(&b)?)),
            None => Ok(None),
        }
    }

    /// Повний запис гри (`None` — не зберігався або вже прибраний).
    pub fn recording(&self, id: &str) -> Result<Option<Vec<u8>>, HistoryError> {
        let id = checked(id)?;
        self.files.read(&format!("{id}{REC_EXT}"))
    }

    /// Видаляє запис разом із повним записом.
    pub fn delete(&self, id: &str) -> Result<(), HistoryError> {
        let id = checked(id)?;
        self.files.remove(&format!("{id}{REC_EXT}"))?;
        self.files.remove(&format!("{id}{ENTRY_EXT}"))
    }

    /// Видаляє всю історію; повертає кількість записів.
    pub fn clear(&self) -> Result<usize, HistoryError> {
        let mut n = 0;
        for name in self.files.list()? {
            let ours = name.ends_with(ENTRY_EXT) || name.ends_with(REC_EXT);
            if ours {
                if name.ends_with(ENTRY_EXT) {
                    n += 1;
                }
                self.files.remove(&name)?;
            }
        }
        Ok(n)
    }

    /// Лишає повні записи лише для `keep` найновіших записів кожної гри; старіші стають
    /// підсумком. Повертає, скільки повних записів прибрано.
    pub fn prune(&self, keep: usize) -> Result<usize, HistoryError> {
        let (entries, _) = self.list()?;
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut removed = 0;
        for e in entries.into_iter().filter(|e| e.has_recording) {
            let n = seen.entry(e.game.to_ascii_lowercase()).or_insert(0);
            *n += 1;
            if *n > keep {
                self.files.remove(&format!("{}{REC_EXT}", e.id))?;
                let mut e = e;
                e.has_recording = false;
                self.files.write(
                    &format!("{}{ENTRY_EXT}", e.id),
                    &serde_json::to_vec_pretty(&e)?,
                )?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::tests::entry;

    fn game_entry(game: &str, at: i64) -> HistoryEntry {
        let mut e = entry(at, 100.0, None);
        e.game = game.into();
        e.id = entry_id(game, at);
        e
    }

    #[test]
    fn save_list_get_delete() {
        let files = MemFiles::default();
        let s = Store::new(&files);
        s.save(&game_entry("re9.exe", 1), Some(b"rec1")).unwrap();
        s.save(&game_entry("re9.exe", 3), None).unwrap();
        s.save(&game_entry("cs2.exe", 2), Some(b"rec2")).unwrap();
        files.write("broken.json", b"{").unwrap();
        files.write("notes.txt", b"x").unwrap();
        let (list, broken) = s.list().unwrap();
        let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["3-re9.exe", "2-cs2.exe", "1-re9.exe"]);
        assert_eq!(broken, 1);
        assert!(list[2].has_recording && !list[0].has_recording);
        assert_eq!(
            s.recording("1-re9.exe").unwrap().as_deref(),
            Some(&b"rec1"[..])
        );
        s.delete("1-re9.exe").unwrap();
        assert!(s.get("1-re9.exe").unwrap().is_none());
        assert!(s.recording("1-re9.exe").unwrap().is_none());
        // Очищення прибирає й пошкоджений запис (тека наша), але не чужі файли.
        assert_eq!(s.clear().unwrap(), 3);
        assert_eq!(files.list().unwrap(), vec!["notes.txt"]);
    }

    #[test]
    fn ids_cannot_escape_the_folder() {
        assert_eq!(entry_id("Some Game!.exe", 5), "5-some_game_.exe");
        let files = MemFiles::default();
        let s = Store::new(&files);
        for bad in ["..\\x", "../x", "a/b", "", ".hidden", "a..b", "C:x"] {
            assert!(matches!(s.get(bad), Err(HistoryError::BadId(_))), "{bad}");
            assert!(s.delete(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn prune_keeps_newest_recordings_per_game() {
        let files = MemFiles::default();
        let s = Store::new(&files);
        for at in 1..=4 {
            s.save(&game_entry("re9.exe", at), Some(b"r")).unwrap();
        }
        s.save(&game_entry("cs2.exe", 1), Some(b"r")).unwrap();
        assert_eq!(s.prune(2).unwrap(), 2);
        let (list, _) = s.list().unwrap();
        let with: Vec<&str> = list
            .iter()
            .filter(|e| e.has_recording)
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(with, ["4-re9.exe", "3-re9.exe", "1-cs2.exe"]);
        assert!(s.recording("1-re9.exe").unwrap().is_none());
        // Підсумки старих записів лишаються.
        assert_eq!(list.len(), 5);
        assert_eq!(s.prune(2).unwrap(), 0);
    }

    #[test]
    fn std_dir_writes_atomically_and_lists_files() {
        let dir = std::env::temp_dir().join(format!("optimax-history-test-{}", std::process::id()));
        let d = StdDir::new(dir.clone()).unwrap();
        d.write("a.json", b"1").unwrap();
        d.write("a.json", b"2").unwrap();
        assert_eq!(d.read("a.json").unwrap().as_deref(), Some(&b"2"[..]));
        assert_eq!(d.list().unwrap(), vec!["a.json"]);
        d.remove("a.json").unwrap();
        d.remove("a.json").unwrap();
        assert!(d.read("a.json").unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
