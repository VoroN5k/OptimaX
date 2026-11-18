//! Доступ бібліотеки до файлів: лише читання конкретних шляхів. Трейт — щоб скан тестувався
//! на моку без реального диска.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// Файлова система для бібліотеки ігор (лише читання).
pub trait LibraryFs: Send + Sync {
    /// Читає текстовий файл (UTF-8).
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    /// Чи це тека. За посиланнями (junction, symlink) іде: бібліотека лише читає, а ігри
    /// часто переносять на інший диск саме через junction.
    fn is_dir(&self, path: &Path) -> bool;
    /// Чи це файл (за посиланнями — як [`LibraryFs::is_dir`]).
    fn is_file(&self, path: &Path) -> bool;
    /// Файли з розширенням `ext` (без крапки, без урахування регістру) безпосередньо в `dir`.
    fn list_files(&self, dir: &Path, ext: &str) -> io::Result<Vec<PathBuf>>;
    /// Безпосередні підтеки `dir`.
    fn list_dirs(&self, dir: &Path) -> io::Result<Vec<PathBuf>>;
    /// Читає файл повністю, якщо він не більший за `max` байтів; інакше помилка.
    fn read_bytes(&self, path: &Path, max: u64) -> io::Result<Vec<u8>>;
}

/// Справжня файлова система.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdFs;

impl LibraryFs for StdFs {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn list_dirs(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                out.push(path);
            }
        }
        out.sort();
        Ok(out)
    }

    fn list_files(&self, dir: &Path, ext: &str) -> io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let matches = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(ext));
            if matches && self.is_file(&path) {
                out.push(path);
            }
        }
        out.sort();
        Ok(out)
    }

    fn read_bytes(&self, path: &Path, max: u64) -> io::Result<Vec<u8>> {
        let len = std::fs::metadata(path)?.len();
        if len > max {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "файл завеликий"));
        }
        std::fs::read(path)
    }
}

/// Ключ мока: `\` як роздільник, нижній регістр, без кінцевого роздільника.
fn mem_key(path: &Path) -> String {
    super::norm_dir(&path.to_string_lossy())
}

/// Файли в пам'яті для тестів (шляхи без урахування регістру, роздільник `\`).
#[derive(Debug, Default, Clone)]
pub struct MemFs {
    files: BTreeMap<String, (PathBuf, Vec<u8>)>,
    dirs: std::collections::BTreeSet<String>,
}

impl MemFs {
    /// Порожня файлова система.
    pub fn new() -> Self {
        Self::default()
    }

    /// Додає файл і всі його батьківські теки.
    pub fn file(&mut self, path: &str, content: impl Into<Vec<u8>>) -> &mut Self {
        let p = PathBuf::from(path);
        self.add_parents(path);
        self.files.insert(mem_key(&p), (p, content.into()));
        self
    }

    /// Додає теку і всі батьківські.
    pub fn dir(&mut self, path: &str) -> &mut Self {
        self.dirs.insert(mem_key(Path::new(path)));
        self.add_parents(path);
        self
    }

    fn add_parents(&mut self, path: &str) {
        let key = super::norm_dir(path);
        let mut cur = key.as_str();
        while let Some(i) = cur.rfind('\\') {
            cur = &cur[..i];
            if cur.is_empty() {
                break;
            }
            self.dirs.insert(cur.to_string());
        }
    }
}

impl LibraryFs for MemFs {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        let bytes = self.read_bytes(path, u64::MAX)?;
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.contains(&mem_key(path))
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(&mem_key(path))
    }

    fn list_files(&self, dir: &Path, ext: &str) -> io::Result<Vec<PathBuf>> {
        let key = mem_key(dir);
        if !self.dirs.contains(&key) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "немає теки"));
        }
        let prefix = format!("{key}\\");
        let suffix = format!(".{}", ext.to_lowercase());
        Ok(self
            .files
            .iter()
            .filter(|(k, _)| {
                k.strip_prefix(&prefix)
                    .is_some_and(|rest| !rest.contains('\\') && rest.ends_with(&suffix))
            })
            .map(|(_, (p, _))| p.clone())
            .collect())
    }

    fn list_dirs(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let key = mem_key(dir);
        if !self.dirs.contains(&key) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "немає теки"));
        }
        let prefix = format!("{key}\\");
        Ok(self
            .dirs
            .iter()
            .filter(|d| {
                d.strip_prefix(&prefix)
                    .is_some_and(|rest| !rest.is_empty() && !rest.contains('\\'))
            })
            .map(PathBuf::from)
            .collect())
    }

    fn read_bytes(&self, path: &Path, max: u64) -> io::Result<Vec<u8>> {
        let (_, bytes) = self
            .files
            .get(&mem_key(path))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "немає файлу"))?;
        if bytes.len() as u64 > max {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "файл завеликий"));
        }
        Ok(bytes.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_lists_only_direct_files_with_extension() {
        let mut fs = MemFs::new();
        fs.file(r"D:\Steam\steamapps\appmanifest_1.acf", "a")
            .file(r"D:\Steam\steamapps\libraryfolders.vdf", "b")
            .file(r"D:\Steam\steamapps\sub\appmanifest_2.acf", "c");
        let files = fs
            .list_files(Path::new(r"d:\steam\steamapps"), "ACF")
            .unwrap();
        assert_eq!(
            files,
            vec![PathBuf::from(r"D:\Steam\steamapps\appmanifest_1.acf")]
        );
        assert!(fs.is_dir(Path::new(r"D:\Steam")));
        assert!(fs.list_files(Path::new(r"X:\none"), "acf").is_err());
    }

    #[test]
    fn mem_lists_direct_subdirs() {
        let mut fs = MemFs::new();
        fs.file(r"C:\c\a\x.jpg", "1")
            .file(r"C:\c\b\d\y.jpg", "2")
            .file(r"C:\c\z.jpg", "3");
        let dirs = fs.list_dirs(Path::new(r"C:\c")).unwrap();
        let names: Vec<String> = dirs
            .iter()
            .map(|d| super::super::norm_dir(&d.to_string_lossy()))
            .collect();
        assert_eq!(names, vec![r"c:\c\a".to_string(), r"c:\c\b".to_string()]);
        assert!(fs.list_dirs(Path::new(r"X:\none")).is_err());
    }

    #[test]
    fn mem_read_bytes_respects_limit() {
        let mut fs = MemFs::new();
        fs.file(r"C:\a.jpg", vec![0u8; 10]);
        assert!(fs.read_bytes(Path::new(r"C:\a.jpg"), 9).is_err());
        assert_eq!(fs.read_bytes(Path::new(r"C:\a.jpg"), 10).unwrap().len(), 10);
    }

    #[test]
    fn std_fs_reads_real_dir() {
        let dir = std::env::temp_dir().join(format!("optimax-libfs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.item"), "{}").unwrap();
        std::fs::write(dir.join("y.txt"), "").unwrap();
        let files = StdFs.list_files(&dir, "item").unwrap();
        assert_eq!(files, vec![dir.join("x.item")]);
        assert!(StdFs.is_dir(&dir));
        assert!(StdFs.is_file(&dir.join("x.item")));
        assert!(StdFs.read_bytes(&dir.join("x.item"), 1).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
