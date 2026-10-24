//! Доступ до файлової системи за трейтом [`FsApi`]: реальна реалізація на `std` і мок у пам'яті.
//!
//! Усі шляхи — нормалізовані рядки (див. [`crate::path`]). Жодна операція не слідує за reparse
//! points: метадані беруться без розіменування, а за каталогом-посиланням обхід не йде.

use crate::path;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Тип об'єкта.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Файл.
    File,
    /// Каталог.
    Dir,
}

/// Метадані без розіменування посилань.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    /// Файл чи каталог.
    pub kind: Kind,
    /// Reparse point (symlink, junction, хмарний файл тощо) — не чіпаємо й не обходимо.
    pub reparse: bool,
    /// Розмір (для файлу).
    pub len: u64,
    /// Найновіша з міток створення / зміни / доступу, Unix-секунди (0 — невідомо).
    pub newest_unix: i64,
}

/// Запис каталогу.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Ім'я.
    pub name: String,
    /// Метадані.
    pub meta: Meta,
}

/// Різновид помилки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsErrorKind {
    /// Немає такого шляху.
    NotFound,
    /// Файл зайнятий іншим процесом.
    Busy,
    /// Немає доступу.
    Denied,
    /// Каталог не порожній.
    NotEmpty,
    /// Ціль уже існує.
    Exists,
    /// Інше.
    Other,
}

/// Помилка файлової операції.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct FsError {
    /// Різновид.
    pub kind: FsErrorKind,
    /// Текст.
    pub message: String,
}

impl FsError {
    /// Нова помилка.
    pub fn new(kind: FsErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Файлова система.
pub trait FsApi: Send + Sync {
    /// Вміст каталогу з метаданими (без розіменування посилань).
    fn read_dir(&self, dir: &str) -> Result<Vec<Entry>, FsError>;
    /// Метадані без розіменування посилань.
    fn metadata(&self, path: &str) -> Result<Meta, FsError>;
    /// Канонічний шлях (розкриває посилання й короткі імена 8.3). Якщо він відрізняється від
    /// нормалізованого вхідного — шлях веде не туди, куди виглядає.
    fn canonicalize(&self, path: &str) -> Result<String, FsError>;
    /// Видаляє файл.
    fn remove_file(&self, path: &str) -> Result<(), FsError>;
    /// Видаляє порожній каталог.
    fn remove_empty_dir(&self, path: &str) -> Result<(), FsError>;
    /// Переміщує файл у межах тому (ціль не повинна існувати).
    fn rename(&self, from: &str, to: &str) -> Result<(), FsError>;
    /// Копіює файл (ціль не повинна існувати). Повертає кількість байтів.
    fn copy_file(&self, from: &str, to: &str) -> Result<u64, FsError>;
    /// Створює каталог разом із батьківськими.
    fn create_dir_all(&self, path: &str) -> Result<(), FsError>;
    /// Читає текстовий файл (`None`, якщо його немає).
    fn read_text(&self, path: &str) -> Result<Option<String>, FsError>;
    /// Атомарно записує текстовий файл (тимчасовий файл + перейменування).
    fn write_text_atomic(&self, path: &str, text: &str) -> Result<(), FsError>;
}

// ---------------------------------------------------------------------------------------------
// Реальна ФС
// ---------------------------------------------------------------------------------------------

/// Файлова система через `std::fs`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdFs;

fn io_err(e: &std::io::Error, what: &str) -> FsError {
    let kind = match e.raw_os_error() {
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        Some(32) | Some(33) => FsErrorKind::Busy,
        // ERROR_DIR_NOT_EMPTY
        Some(145) => FsErrorKind::NotEmpty,
        _ => match e.kind() {
            std::io::ErrorKind::NotFound => FsErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => FsErrorKind::Denied,
            std::io::ErrorKind::AlreadyExists => FsErrorKind::Exists,
            _ => FsErrorKind::Other,
        },
    };
    FsError::new(kind, format!("{what}: {e}"))
}

fn unix(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

fn to_meta(m: &std::fs::Metadata) -> Meta {
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    };
    #[cfg(not(windows))]
    let reparse = m.file_type().is_symlink();
    Meta {
        kind: if m.is_dir() { Kind::Dir } else { Kind::File },
        reparse: reparse || m.file_type().is_symlink(),
        len: if m.is_dir() { 0 } else { m.len() },
        newest_unix: unix(m.created())
            .max(unix(m.modified()))
            .max(unix(m.accessed())),
    }
}

impl FsApi for StdFs {
    fn read_dir(&self, dir: &str) -> Result<Vec<Entry>, FsError> {
        let rd = std::fs::read_dir(dir).map_err(|e| io_err(&e, dir))?;
        let mut out = Vec::new();
        for entry in rd {
            let Ok(entry) = entry else { continue };
            // На Windows метадані запису беруться з даних FindNextFile — без відкриття файлу
            // й без розіменування посилань.
            let Ok(meta) = entry.metadata() else { continue };
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                // Ім'я не в UTF-16, яке можна передати в UTF-8, — пропускаємо, не вгадуємо.
                continue;
            };
            out.push(Entry {
                name,
                meta: to_meta(&meta),
            });
        }
        Ok(out)
    }

    fn metadata(&self, path: &str) -> Result<Meta, FsError> {
        std::fs::symlink_metadata(path)
            .map(|m| to_meta(&m))
            .map_err(|e| io_err(&e, path))
    }

    fn canonicalize(&self, path: &str) -> Result<String, FsError> {
        let c = std::fs::canonicalize(path).map_err(|e| io_err(&e, path))?;
        let s = c
            .to_str()
            .ok_or_else(|| FsError::new(FsErrorKind::Other, format!("{path}: не UTF-8")))?;
        // `\\?\C:\…` → `C:\…`; `\\?\UNC\…` лишається як є й не збіжеться з локальним шляхом.
        Ok(path::normalize(s).unwrap_or_else(|_| s.to_string()))
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        std::fs::remove_file(path).map_err(|e| io_err(&e, path))
    }

    fn remove_empty_dir(&self, path: &str) -> Result<(), FsError> {
        std::fs::remove_dir(path).map_err(|e| io_err(&e, path))
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(FsError::new(FsErrorKind::Exists, to.to_string()));
        }
        std::fs::rename(from, to).map_err(|e| io_err(&e, from))
    }

    fn copy_file(&self, from: &str, to: &str) -> Result<u64, FsError> {
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(FsError::new(FsErrorKind::Exists, to.to_string()));
        }
        std::fs::copy(from, to).map_err(|e| io_err(&e, from))
    }

    fn create_dir_all(&self, path: &str) -> Result<(), FsError> {
        std::fs::create_dir_all(path).map_err(|e| io_err(&e, path))
    }

    fn read_text(&self, path: &str) -> Result<Option<String>, FsError> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(&e, path)),
        }
    }

    fn write_text_atomic(&self, path: &str, text: &str) -> Result<(), FsError> {
        use std::io::Write;
        if let Some(dir) = path::parent(path) {
            self.create_dir_all(dir)?;
        }
        let tmp = format!("{path}.tmp");
        {
            let mut f = std::fs::File::create(&tmp).map_err(|e| io_err(&e, &tmp))?;
            f.write_all(text.as_bytes()).map_err(|e| io_err(&e, &tmp))?;
            f.sync_all().map_err(|e| io_err(&e, &tmp))?;
        }
        // rename на Windows замінює файл атомарно (MoveFileEx з REPLACE_EXISTING).
        std::fs::rename(&tmp, path).map_err(|e| io_err(&e, path))
    }
}

// ---------------------------------------------------------------------------------------------
// Мок у пам'яті
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum MemKind {
    File {
        len: u64,
        text: Option<String>,
    },
    Dir,
    /// Symlink або junction на `target`.
    Link {
        target: String,
        dir: bool,
    },
}

#[derive(Debug, Clone)]
struct MemNode {
    path: String,
    kind: MemKind,
    newest_unix: i64,
    locked: bool,
}

#[derive(Debug, Default)]
struct MemState {
    nodes: BTreeMap<String, MemNode>,
    /// Короткі імена 8.3: `c:\progra~1` → `C:\Program Files`.
    aliases: HashMap<String, String>,
    /// Журнал змінювальних операцій (для перевірки «сухий прогін нічого не змінює»).
    mutations: Vec<String>,
}

/// Файлова система в пам'яті для тестів: каталоги, файли, посилання, короткі імена, зайняті файли.
/// Шляхи — у вигляді Windows незалежно від ОС, на якій ідуть тести.
#[derive(Debug, Default)]
pub struct MemFs {
    state: Mutex<MemState>,
}

fn nf(path: &str) -> FsError {
    FsError::new(FsErrorKind::NotFound, path.to_string())
}

impl MemFs {
    /// Порожня ФС.
    pub fn new() -> Self {
        Self::default()
    }

    fn st(&self) -> MutexGuard<'_, MemState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn insert(&self, path: &str, kind: MemKind, newest_unix: i64) {
        let Ok(p) = path::normalize(path) else {
            return;
        };
        let mut st = self.st();
        // Батьківські каталоги створюються неявно.
        let mut cur = p.as_str();
        while let Some(parent) = path::parent(cur) {
            st.nodes
                .entry(path::key(parent))
                .or_insert_with(|| MemNode {
                    path: parent.to_string(),
                    kind: MemKind::Dir,
                    newest_unix: 0,
                    locked: false,
                });
            cur = parent;
        }
        st.nodes.insert(
            path::key(&p),
            MemNode {
                path: p,
                kind,
                newest_unix,
                locked: false,
            },
        );
    }

    /// Додає каталог.
    pub fn add_dir(&self, path: &str) -> &Self {
        self.insert(path, MemKind::Dir, 0);
        self
    }

    /// Додає каталог з міткою часу.
    pub fn add_dir_at(&self, path: &str, newest_unix: i64) -> &Self {
        self.insert(path, MemKind::Dir, newest_unix);
        self
    }

    /// Додає файл розміром `len` з міткою часу `newest_unix`.
    pub fn add_file(&self, path: &str, len: u64, newest_unix: i64) -> &Self {
        self.insert(path, MemKind::File { len, text: None }, newest_unix);
        self
    }

    /// Додає symlink / junction `path` → `target`.
    pub fn add_link(&self, path: &str, target: &str, dir: bool) -> &Self {
        let target = path::normalize(target).unwrap_or_default();
        self.insert(path, MemKind::Link { target, dir }, 0);
        self
    }

    /// Додає коротке ім'я 8.3 для існуючого шляху.
    pub fn add_alias(&self, alias: &str, real: &str) -> &Self {
        if let (Ok(a), Ok(r)) = (path::normalize(alias), path::normalize(real)) {
            self.st().aliases.insert(path::key(&a), r);
        }
        self
    }

    /// Позначає файл зайнятим (видалення й переміщення повертають `Busy`).
    pub fn lock_file(&self, path: &str) -> &Self {
        if let Some(n) = self.st().nodes.get_mut(&path::key(path)) {
            n.locked = true;
        }
        self
    }

    /// Чи існує шлях.
    pub fn exists(&self, path: &str) -> bool {
        self.st().nodes.contains_key(&path::key(path))
    }

    /// Розмір файлу.
    pub fn len_of(&self, path: &str) -> Option<u64> {
        match self.st().nodes.get(&path::key(path))?.kind {
            MemKind::File { len, .. } => Some(len),
            _ => None,
        }
    }

    /// Кількість змінювальних операцій.
    pub fn mutation_count(&self) -> usize {
        self.st().mutations.len()
    }

    /// Змінювальні операції (для діагностики тестів).
    pub fn mutations(&self) -> Vec<String> {
        self.st().mutations.clone()
    }

    fn children(st: &MemState, dir_key: &str) -> Vec<MemNode> {
        st.nodes
            .values()
            .filter(|n| path::parent(&n.path).map(path::key).as_deref() == Some(dir_key))
            .cloned()
            .collect()
    }

    fn meta_of(n: &MemNode) -> Meta {
        match &n.kind {
            MemKind::File { len, .. } => Meta {
                kind: Kind::File,
                reparse: false,
                len: *len,
                newest_unix: n.newest_unix,
            },
            MemKind::Dir => Meta {
                kind: Kind::Dir,
                reparse: false,
                len: 0,
                newest_unix: n.newest_unix,
            },
            MemKind::Link { dir, .. } => Meta {
                kind: if *dir { Kind::Dir } else { Kind::File },
                reparse: true,
                len: 0,
                newest_unix: n.newest_unix,
            },
        }
    }

    /// Розкриває посилання й короткі імена (як справжня ОС при відкритті шляху).
    fn resolve(st: &MemState, p: &str) -> Result<String, FsError> {
        let norm =
            path::normalize(p).map_err(|e| FsError::new(FsErrorKind::Other, e.to_string()))?;
        let drive = norm.get(..3).unwrap_or_default().to_string();
        let mut cur = drive;
        for comp in path::components(&norm) {
            let mut next = path::join(&cur, comp);
            for _ in 0..16 {
                if let Some(real) = st.aliases.get(&path::key(&next)) {
                    next = real.clone();
                    continue;
                }
                match st.nodes.get(&path::key(&next)) {
                    Some(MemNode {
                        kind: MemKind::Link { target, .. },
                        ..
                    }) => next = target.clone(),
                    Some(n) => {
                        next = n.path.clone();
                        break;
                    }
                    None => return Err(nf(p)),
                }
            }
            cur = next;
        }
        Ok(cur)
    }

    fn mutate(st: &mut MemState, op: String) {
        st.mutations.push(op);
    }
}

impl FsApi for MemFs {
    fn read_dir(&self, dir: &str) -> Result<Vec<Entry>, FsError> {
        let st = self.st();
        // Як справжня ОС: обхід каталогу-посилання веде в ціль. Тому код, що «випадково» зайде
        // в junction, у тестах побачить чужі файли — і тест це зловить.
        let real = Self::resolve(&st, dir)?;
        match st.nodes.get(&path::key(&real)) {
            Some(MemNode {
                kind: MemKind::Dir, ..
            }) => {}
            Some(_) => {
                return Err(FsError::new(
                    FsErrorKind::Other,
                    format!("{dir}: не каталог"),
                ))
            }
            None => return Err(nf(dir)),
        }
        Ok(Self::children(&st, &path::key(&real))
            .iter()
            .map(|n| Entry {
                name: path::file_name(&n.path).to_string(),
                meta: Self::meta_of(n),
            })
            .collect())
    }

    fn metadata(&self, p: &str) -> Result<Meta, FsError> {
        let st = self.st();
        let key = path::key(p);
        let key = st.aliases.get(&key).map(|r| path::key(r)).unwrap_or(key);
        st.nodes.get(&key).map(Self::meta_of).ok_or_else(|| nf(p))
    }

    fn canonicalize(&self, p: &str) -> Result<String, FsError> {
        Self::resolve(&self.st(), p)
    }

    fn remove_file(&self, p: &str) -> Result<(), FsError> {
        let mut st = self.st();
        let key = path::key(p);
        match st.nodes.get(&key) {
            Some(n) if n.locked => Err(FsError::new(FsErrorKind::Busy, p.to_string())),
            Some(MemNode {
                kind: MemKind::File { .. } | MemKind::Link { dir: false, .. },
                ..
            }) => {
                st.nodes.remove(&key);
                Self::mutate(&mut st, format!("remove_file {p}"));
                Ok(())
            }
            Some(_) => Err(FsError::new(FsErrorKind::Other, format!("{p}: не файл"))),
            None => Err(nf(p)),
        }
    }

    fn remove_empty_dir(&self, p: &str) -> Result<(), FsError> {
        let mut st = self.st();
        let key = path::key(p);
        match st.nodes.get(&key) {
            Some(MemNode {
                kind: MemKind::Dir, ..
            }) => {
                if !Self::children(&st, &key).is_empty() {
                    return Err(FsError::new(FsErrorKind::NotEmpty, p.to_string()));
                }
                st.nodes.remove(&key);
                Self::mutate(&mut st, format!("remove_dir {p}"));
                Ok(())
            }
            Some(_) => Err(FsError::new(FsErrorKind::Other, format!("{p}: не каталог"))),
            None => Err(nf(p)),
        }
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        let mut st = self.st();
        let (fk, tk) = (path::key(from), path::key(to));
        let node = st.nodes.get(&fk).cloned().ok_or_else(|| nf(from))?;
        if node.locked {
            return Err(FsError::new(FsErrorKind::Busy, from.to_string()));
        }
        if st.nodes.contains_key(&tk) {
            return Err(FsError::new(FsErrorKind::Exists, to.to_string()));
        }
        let parent_ok = path::parent(to).is_some_and(|p| {
            matches!(
                st.nodes.get(&path::key(p)),
                Some(MemNode {
                    kind: MemKind::Dir,
                    ..
                })
            )
        });
        if !parent_ok {
            return Err(nf(to));
        }
        if !matches!(node.kind, MemKind::File { .. }) {
            return Err(FsError::new(FsErrorKind::Other, "мок переміщує лише файли"));
        }
        st.nodes.remove(&fk);
        st.nodes.insert(
            tk,
            MemNode {
                path: to.to_string(),
                ..node
            },
        );
        Self::mutate(&mut st, format!("rename {from} -> {to}"));
        Ok(())
    }

    fn copy_file(&self, from: &str, to: &str) -> Result<u64, FsError> {
        let node = self
            .st()
            .nodes
            .get(&path::key(from))
            .cloned()
            .ok_or_else(|| nf(from))?;
        let MemKind::File { len, .. } = node.kind else {
            return Err(FsError::new(FsErrorKind::Other, format!("{from}: не файл")));
        };
        if self.exists(to) {
            return Err(FsError::new(FsErrorKind::Exists, to.to_string()));
        }
        if !path::parent(to).is_some_and(|p| self.exists(p)) {
            return Err(nf(to));
        }
        self.insert(to, node.kind.clone(), node.newest_unix);
        Self::mutate(&mut self.st(), format!("copy {from} -> {to}"));
        Ok(len)
    }

    fn create_dir_all(&self, p: &str) -> Result<(), FsError> {
        if !self.exists(p) {
            self.insert(p, MemKind::Dir, 0);
            Self::mutate(&mut self.st(), format!("mkdir {p}"));
        }
        Ok(())
    }

    fn read_text(&self, p: &str) -> Result<Option<String>, FsError> {
        match self.st().nodes.get(&path::key(p)) {
            Some(MemNode {
                kind: MemKind::File { text, .. },
                ..
            }) => Ok(Some(text.clone().unwrap_or_default())),
            Some(_) => Err(FsError::new(FsErrorKind::Other, format!("{p}: не файл"))),
            None => Ok(None),
        }
    }

    fn write_text_atomic(&self, p: &str, text: &str) -> Result<(), FsError> {
        self.insert(
            p,
            MemKind::File {
                len: text.len() as u64,
                text: Some(text.to_string()),
            },
            0,
        );
        Self::mutate(&mut self.st(), format!("write {p}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memfs_resolves_links_and_aliases() {
        let fs = MemFs::new();
        fs.add_file(r"C:\Windows\System32\kernel32.dll", 10, 0)
            .add_link(r"C:\Temp\evil", r"C:\Windows\System32", true)
            .add_dir(r"C:\Program Files\App")
            .add_alias(r"C:\PROGRA~1", r"C:\Program Files");
        assert_eq!(
            fs.canonicalize(r"C:\Temp\evil\kernel32.dll").unwrap(),
            r"C:\Windows\System32\kernel32.dll"
        );
        assert_eq!(
            fs.canonicalize(r"C:\PROGRA~1\App").unwrap(),
            r"C:\Program Files\App"
        );
        assert!(fs.metadata(r"C:\Temp\evil").unwrap().reparse);
        // Обхід посилання веде в ціль — як у справжній ОС.
        assert_eq!(
            fs.read_dir(r"C:\Temp\evil").unwrap()[0].name,
            "kernel32.dll"
        );
    }

    #[test]
    fn memfs_locked_file_is_busy() {
        let fs = MemFs::new();
        fs.add_file(r"C:\Temp\a.tmp", 1, 0)
            .lock_file(r"C:\Temp\a.tmp");
        assert_eq!(
            fs.remove_file(r"C:\Temp\a.tmp").unwrap_err().kind,
            FsErrorKind::Busy
        );
        assert_eq!(fs.mutation_count(), 0);
    }
}
