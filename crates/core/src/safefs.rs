//! Файлові операції, стійкі до підміни шляху (junction, symlink, коротке ім'я 8.3).
//!
//! Програма працює з правами адміністратора, а чистить теки, куди може писати будь-який процес
//! користувача. Операція «за шляхом» (`DeleteFileW`, `MoveFileExW`) розкриває посилання в
//! проміжних теках у момент виклику, тож перевірку шляху заздалегідь можна обійти підміною теки.
//! Тому тут кожен об'єкт спершу **відкривається** без розкриття останнього посилання, справжній
//! шлях відкритого об'єкта звіряється з очікуваним, і лише тоді операція виконується над **цим
//! самим дескриптором**.
//!
//! Шляхи — абсолютні локальні (`C:\…`, без `\\?\`), порівняння без урахування регістру.

use std::fmt;
use std::path::Path;

#[cfg(windows)]
mod windows;

/// Помилка безпечної файлової операції.
#[derive(Debug)]
pub enum SafeFsError {
    /// Справжній шлях відкритого об'єкта не збігається з очікуваним (посилання по дорозі).
    Redirected {
        /// Очікуваний шлях.
        expected: String,
        /// Куди насправді веде.
        actual: String,
    },
    /// Сам об'єкт — reparse point (symlink, junction).
    Reparse(String),
    /// Об'єкт не того типу (каталог замість файлу чи навпаки).
    WrongKind(String),
    /// Помилка вводу-виводу.
    Io(std::io::Error),
}

impl fmt::Display for SafeFsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Redirected { expected, actual } => {
                write!(f, "{expected}: шлях перенаправлено на {actual}")
            }
            Self::Reparse(p) => write!(f, "{p}: reparse point"),
            Self::WrongKind(p) => write!(f, "{p}: неочікуваний тип об'єкта"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SafeFsError {}

impl From<std::io::Error> for SafeFsError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Порівнює справжній шлях (як його повертає система) з очікуваним.
fn same_path(actual: &str, expected: &str) -> bool {
    let strip = |s: &str| {
        s.strip_prefix(r"\\?\")
            .unwrap_or(s)
            .trim_end_matches('\\')
            .to_lowercase()
    };
    // `\\?\UNC\…` після зрізання префікса не схожий на `C:\…` — збігу не буде.
    strip(actual) == strip(expected)
}

/// Видаляє файл, якщо він лежить саме за цим шляхом і сам не є посиланням.
pub fn delete_file(path: &Path) -> Result<(), SafeFsError> {
    imp::delete(path, false)
}

/// Видаляє порожній каталог, якщо він лежить саме за цим шляхом і сам не є посиланням.
pub fn delete_empty_dir(path: &Path) -> Result<(), SafeFsError> {
    imp::delete(path, true)
}

/// Переміщує файл у межах тому. І джерело, і тека призначення перевіряються за дескриптором;
/// наявний файл призначення не перезаписується.
pub fn move_file(from: &Path, to: &Path) -> Result<(), SafeFsError> {
    imp::move_file(from, to)
}

/// Копіює вміст файлу (і час зміни) у новий файл; наявний не перезаписується. Джерело читається
/// з перевіреного дескриптора; створений файл, що опинився не там, одразу видаляється.
pub fn copy_file(from: &Path, to: &Path) -> Result<u64, SafeFsError> {
    imp::copy_file(from, to)
}

/// Читає файл цілком (не більше `limit` байтів), лише якщо він лежить саме за цим шляхом і сам
/// не є посиланням. `Ok(None)` — файлу немає.
pub fn read_limited(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, SafeFsError> {
    match imp::read_limited(path, limit) {
        Err(SafeFsError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        other => other.map(Some),
    }
}

/// Відкриває каталог для читання дескриптора безпеки з тими самими перевірками шляху.
#[cfg(windows)]
pub(crate) fn open_dir_for_security(path: &Path) -> Result<std::fs::File, SafeFsError> {
    imp::open_dir_for_security(path)
}

#[cfg(windows)]
use self::windows as imp;

/// Поза Windows (збірка й тести на інших ОС): звичайні операції з перевіркою посилань.
#[cfg(not(windows))]
mod imp {
    use super::SafeFsError;
    use std::path::Path;

    fn check(path: &Path, dir: bool) -> Result<(), SafeFsError> {
        let m = std::fs::symlink_metadata(path)?;
        if m.file_type().is_symlink() {
            return Err(SafeFsError::Reparse(path.display().to_string()));
        }
        if m.is_dir() != dir {
            return Err(SafeFsError::WrongKind(path.display().to_string()));
        }
        let parent = path.parent().unwrap_or(path);
        let canon = std::fs::canonicalize(parent)?;
        if canon != parent {
            return Err(SafeFsError::Redirected {
                expected: parent.display().to_string(),
                actual: canon.display().to_string(),
            });
        }
        Ok(())
    }

    pub(super) fn delete(path: &Path, dir: bool) -> Result<(), SafeFsError> {
        check(path, dir)?;
        if dir {
            std::fs::remove_dir(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    pub(super) fn move_file(from: &Path, to: &Path) -> Result<(), SafeFsError> {
        check(from, false)?;
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into());
        }
        std::fs::rename(from, to)?;
        Ok(())
    }

    pub(super) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, SafeFsError> {
        use std::io::Read;
        check(path, false)?;
        let mut out = Vec::new();
        std::fs::File::open(path)?
            .take(limit + 1)
            .read_to_end(&mut out)?;
        if out.len() as u64 > limit {
            return Err(SafeFsError::WrongKind(path.display().to_string()));
        }
        Ok(out)
    }

    pub(super) fn copy_file(from: &Path, to: &Path) -> Result<u64, SafeFsError> {
        check(from, false)?;
        let mut src = std::fs::File::open(from)?;
        let mut dst = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        Ok(std::io::copy(&mut src, &mut dst)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_path_comparison() {
        assert!(same_path(
            r"\\?\C:\Users\A\Temp\x.tmp",
            r"C:\users\a\temp\X.TMP"
        ));
        assert!(same_path(r"\\?\C:\Temp\", r"C:\Temp"));
        assert!(!same_path(
            r"\\?\C:\Windows\System32\x.dll",
            r"C:\Temp\evil\x.dll"
        ));
        assert!(!same_path(r"\\?\UNC\srv\share\x", r"C:\x"));
    }
}
