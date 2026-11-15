//! Файл журналу з обмеженням розміру. Служба працює тижнями без перезапуску, тож обрізати журнал
//! лише на старті мало: коли файл досягає межі, він стає `*.old.log` (попередній такий видаляється)
//! і запис іде в новий. На диску — не більше двох файлів, тобто ≤ 2 × межа.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Журнал, що не росте понад `max` байтів (плюс один попередній файл такого ж розміру).
#[derive(Debug)]
pub struct CappedLog {
    path: PathBuf,
    file: Option<File>,
    len: u64,
    max: u64,
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

impl CappedLog {
    /// Відкриває (створює) журнал `path`. Якщо він уже більший за `max` — одразу переносить його
    /// в попередній.
    pub fn open(path: &Path, max: u64) -> io::Result<Self> {
        let file = open_append(path)?;
        let len = file.metadata()?.len();
        let mut log = Self {
            path: path.to_path_buf(),
            file: Some(file),
            len,
            max,
        };
        if len > max {
            log.rotate()?;
        }
        Ok(log)
    }

    /// Шлях попереднього файлу: `service.log` → `service.old.log`.
    pub fn old_path(path: &Path) -> PathBuf {
        path.with_extension("old.log")
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let old = Self::old_path(&self.path);
        let _ = std::fs::remove_file(&old);
        if std::fs::rename(&self.path, &old).is_err() {
            // Не вийшло перейменувати — просто починаємо файл спочатку.
            let _ = std::fs::remove_file(&self.path);
        }
        self.file = Some(open_append(&self.path)?);
        self.len = 0;
        Ok(())
    }
}

impl Write for CappedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.len > 0 && self.len + buf.len() as u64 > self.max {
            self.rotate()?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("журнал служби не відкрито"))?;
        let n = file.write(buf)?;
        self.len += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "optimax-log-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn size(p: &Path) -> u64 {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    }

    #[test]
    fn never_grows_past_two_files() {
        let dir = Tmp::new("cap");
        let path = dir.0.join("service.log");
        let mut log = CappedLog::open(&path, 1000).unwrap();
        let line = [b'x'; 99];
        for _ in 0..500 {
            log.write_all(&line).unwrap();
            log.write_all(b"\n").unwrap();
        }
        log.flush().unwrap();
        assert!(size(&path) <= 1000, "поточний {}", size(&path));
        let old = CappedLog::old_path(&path);
        assert_eq!(old.file_name().unwrap(), "service.old.log");
        assert!(size(&old) <= 1000, "попередній {}", size(&old));
        let files = std::fs::read_dir(&dir.0).unwrap().count();
        assert_eq!(files, 2);
    }

    #[test]
    fn oversized_log_is_moved_aside_on_open() {
        let dir = Tmp::new("open");
        let path = dir.0.join("service.log");
        std::fs::write(&path, vec![b'y'; 5000]).unwrap();
        let mut log = CappedLog::open(&path, 1000).unwrap();
        log.write_all(b"new\n").unwrap();
        log.flush().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new\n");
        assert_eq!(size(&CappedLog::old_path(&path)), 5000);
    }
}
