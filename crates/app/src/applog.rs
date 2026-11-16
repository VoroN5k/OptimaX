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

/// Файл журналу вікна програми.
pub const APP: &str = "app.log";
/// Файл журналу агента в треї (окремий: два процеси не ділять один файл з обрізанням).
pub const AGENT: &str = "agent.log";

/// Шлях до файлу журналу `name` ([`APP`] чи [`AGENT`]).
pub fn path_of(name: &str) -> Option<PathBuf> {
    optimax_core::datadir::ui_dir()
        .ok()
        .map(|d| d.join("logs").join(name))
}

/// Відкриває журнал `name` для запису (`None` — тека недоступна; тоді журналу у файлі немає).
pub fn open_file(name: &str) -> Option<CappedLog> {
    let p = path_of(name)?;
    std::fs::create_dir_all(p.parent()?).ok()?;
    CappedLog::open(&p, MAX).ok()
}

/// Відкриває журнал вікна програми.
pub fn open() -> Option<CappedLog> {
    open_file(APP)
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

/// Останні рядки журналу `name` (попередній файл + поточний) для звіту.
pub fn tail_of(name: &str, n: usize) -> Vec<String> {
    let Some(p) = path_of(name) else {
        return Vec::new();
    };
    let read = |p: &std::path::Path| {
        std::fs::read(p)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    };
    last_lines(&[read(&CappedLog::old_path(&p)), read(&p)], n)
}

/// Останні рядки журналу вікна програми.
pub fn tail(n: usize) -> Vec<String> {
    tail_of(APP, n)
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
