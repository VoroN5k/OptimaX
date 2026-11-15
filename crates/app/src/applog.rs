//! Журнал програми у файлі (`logs\app.log` у теці даних інтерфейсу): щоб, коли в когось щось
//! зламалось, було що подивитись. Той самий обмежений файл, що й у служби
//! (`optimax_service::logfile::CappedLog`): щонайбільше два файли по [`MAX`] байтів.
//!
//! Без прав адміністратора — `%LOCALAPPDATA%\OptimaX\User\logs`, з правами — захищена тека
//! (`datadir::ui_dir`: процес з правами в теку користувача не пише). Нічого не надсилається:
//! останні рядки потрапляють лише у «Звіт для відгуку», який користувач надсилає сам.

use optimax_service::logfile::CappedLog;
use std::path::PathBuf;

/// Межа одного файлу журналу.
pub const MAX: u64 = 2 * 1024 * 1024;
/// Скільки останніх рядків іде у звіт для відгуку.
pub const REPORT_LINES: usize = 300;

/// Шлях до файлу журналу програми.
pub fn path() -> Option<PathBuf> {
    optimax_core::datadir::ui_dir()
        .ok()
        .map(|d| d.join("logs").join("app.log"))
}

/// Відкриває журнал для запису (`None` — тека недоступна; тоді журнал лише в інтерфейсі).
pub fn open() -> Option<CappedLog> {
    let p = path()?;
    std::fs::create_dir_all(p.parent()?).ok()?;
    CappedLog::open(&p, MAX).ok()
}

/// Останні `n` рядків з кількох текстів (від старішого до новішого), без порожніх.
pub fn last_lines(texts: &[String], n: usize) -> Vec<String> {
    let all: Vec<&str> = texts
        .iter()
        .flat_map(|t| t.lines())
        .filter(|l| !l.trim().is_empty())
        .collect();
    all[all.len().saturating_sub(n)..]
        .iter()
        .map(|l| (*l).to_string())
        .collect()
}

/// Останні рядки журналу програми (попередній файл + поточний) для звіту.
pub fn tail(n: usize) -> Vec<String> {
    let Some(p) = path() else {
        return Vec::new();
    };
    let read = |p: &std::path::Path| {
        std::fs::read(p)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    };
    last_lines(&[read(&CappedLog::old_path(&p)), read(&p)], n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_newest_lines_across_files() {
        let old = "a\nb\n\nc\n".to_string();
        let cur = "d\ne\n".to_string();
        assert_eq!(last_lines(&[old.clone(), cur.clone()], 3), ["c", "d", "e"]);
        assert_eq!(last_lines(&[old, cur], 100), ["a", "b", "c", "d", "e"]);
        assert!(last_lines(&[String::new()], 5).is_empty());
    }
}
