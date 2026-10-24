//! Жорсткий чорний список шляхів. Зашитий у код; правила бази не можуть його перекрити.
//!
//! Дві зони:
//! - **заборонені** — нічого всередині не видаляється ніколи (System32, WinSxS, Installer,
//!   Program Files, особисті теки користувачів). Правило з вердиктом 🔴 може на них вказувати лише
//!   для показу розміру;
//! - **контейнери** — сам каталог ніколи не видаляється й не може бути коренем правила (корені
//!   дисків, `Windows`, `ProgramData`, профілі, `AppData`); видаляти можна лише всередині явного
//!   кореня правила.

use crate::pattern::{Env, Pattern};

/// Чому шлях під захистом.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    /// Системні файли Windows.
    System,
    /// Встановлені програми та ігри.
    Programs,
    /// Особисті файли користувача.
    UserFiles,
}

const FORBIDDEN: &[(&str, Zone)] = &[
    (r"%SystemRoot%\System32", Zone::System),
    (r"%SystemRoot%\SysWOW64", Zone::System),
    (r"%SystemRoot%\WinSxS", Zone::System),
    (r"%SystemRoot%\Installer", Zone::System),
    (r"%SystemRoot%\servicing", Zone::System),
    (r"%ProgramFiles%", Zone::Programs),
    (r"%ProgramFiles(x86)%", Zone::Programs),
    (r"%ProgramW6432%", Zone::Programs),
    (r"%SystemDrive%\Users\*\Documents", Zone::UserFiles),
    (r"%SystemDrive%\Users\*\Desktop", Zone::UserFiles),
    (r"%SystemDrive%\Users\*\Pictures", Zone::UserFiles),
    (r"%SystemDrive%\Users\*\Videos", Zone::UserFiles),
    (r"%SystemDrive%\Users\*\Music", Zone::UserFiles),
    (r"%SystemDrive%\Users\*\Downloads", Zone::UserFiles),
    (r"%SystemDrive%\Users\*\OneDrive", Zone::UserFiles),
    (r"%USERPROFILE%\Documents", Zone::UserFiles),
    (r"%USERPROFILE%\Desktop", Zone::UserFiles),
    (r"%USERPROFILE%\Pictures", Zone::UserFiles),
    (r"%USERPROFILE%\Videos", Zone::UserFiles),
    (r"%USERPROFILE%\Music", Zone::UserFiles),
    (r"%USERPROFILE%\Downloads", Zone::UserFiles),
    (r"%OneDrive%", Zone::UserFiles),
];

const CONTAINERS: &[&str] = &[
    r"%SystemRoot%",
    r"%ProgramData%",
    r"%SystemDrive%\Users",
    r"%SystemDrive%\Users\*",
    r"%SystemDrive%\Users\*\AppData",
    r"%SystemDrive%\Users\*\AppData\Local",
    r"%SystemDrive%\Users\*\AppData\LocalLow",
    r"%SystemDrive%\Users\*\AppData\Roaming",
    r"%USERPROFILE%",
    r"%LOCALAPPDATA%",
    r"%APPDATA%",
];

/// Чорний список, розгорнутий для цієї системи.
#[derive(Debug, Clone)]
pub struct Guard {
    forbidden: Vec<(Pattern, Zone)>,
    containers: Vec<Pattern>,
}

impl Guard {
    /// Будує список для змінних середовища (невідомі змінні пропускаються).
    pub fn new(env: &Env) -> Self {
        Self {
            forbidden: FORBIDDEN
                .iter()
                .filter_map(|(p, z)| Pattern::parse(p, env).ok().map(|p| (p, *z)))
                .collect(),
            containers: CONTAINERS
                .iter()
                .filter_map(|p| Pattern::parse(p, env).ok())
                .collect(),
        }
    }

    /// Шлях у забороненій зоні (або дорівнює її кореню).
    pub fn forbidden_zone(&self, p: &str) -> Option<Zone> {
        self.forbidden
            .iter()
            .find(|(pat, _)| pat.contains(p))
            .map(|(_, z)| *z)
    }

    /// Шлях — контейнер або корінь диска: сам по собі не видаляється.
    pub fn is_container(&self, p: &str) -> bool {
        crate::path::is_drive_root(p) || self.containers.iter().any(|c| c.equals(p))
    }

    /// Чи можна видаляти цей об'єкт (без урахування правил).
    pub fn allows_delete(&self, p: &str) -> bool {
        self.forbidden_zone(p).is_none() && !self.is_container(p)
    }

    /// Шаблон правила (не 🔴) порушує чорний список: лежить у забороненій зоні, охоплює її або
    /// збігається з контейнером.
    pub fn rejects_rule_root(&self, root: &Pattern) -> Option<&'static str> {
        if root.is_empty() {
            return Some("корінь диска");
        }
        if self.forbidden.iter().any(|(f, _)| f.covers(root)) {
            return Some("лежить у забороненій зоні");
        }
        if self.forbidden.iter().any(|(f, _)| root.covers(f)) {
            return Some("охоплює заборонену зону");
        }
        if self
            .containers
            .iter()
            .any(|c| c.len() == root.len() && c.overlaps(root))
        {
            return Some("збігається з системним контейнером");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> (Guard, Env) {
        let env = Env::for_tests();
        (Guard::new(&env), env)
    }

    #[test]
    fn forbidden_zones() {
        let (g, _) = guard();
        assert_eq!(
            g.forbidden_zone(r"C:\Windows\System32\drivers\x.sys"),
            Some(Zone::System)
        );
        assert_eq!(g.forbidden_zone(r"c:\windows\winsxs"), Some(Zone::System));
        assert_eq!(
            g.forbidden_zone(r"C:\Windows\Installer\a.msi"),
            Some(Zone::System)
        );
        assert_eq!(
            g.forbidden_zone(r"C:\Program Files\Game\a.pak"),
            Some(Zone::Programs)
        );
        assert_eq!(
            g.forbidden_zone(r"C:\Program Files (x86)\Steam"),
            Some(Zone::Programs)
        );
        assert_eq!(
            g.forbidden_zone(r"C:\Users\Other\Documents\a.docx"),
            Some(Zone::UserFiles)
        );
        assert_eq!(
            g.forbidden_zone(r"C:\Users\Test\Downloads\setup.exe"),
            Some(Zone::UserFiles)
        );
        assert_eq!(g.forbidden_zone(r"C:\Windows\Temp\a.tmp"), None);
    }

    #[test]
    fn containers_never_deleted() {
        let (g, _) = guard();
        for p in [
            r"C:\",
            r"D:\",
            r"C:\Windows",
            r"C:\ProgramData",
            r"C:\Users",
            r"C:\Users\Test",
            r"C:\Users\Other\AppData\Local",
        ] {
            assert!(!g.allows_delete(p), "{p}");
        }
        assert!(g.allows_delete(r"C:\Users\Test\AppData\Local\Temp\x.tmp"));
    }

    #[test]
    fn rule_roots_are_checked() {
        let (g, env) = guard();
        let bad = |s: &str| g.rejects_rule_root(&Pattern::parse(s, &env).unwrap());
        assert!(bad(r"%SystemRoot%\System32\LogFiles").is_some());
        assert!(bad(r"%SystemDrive%\Users\*").is_some());
        assert!(bad(r"%LOCALAPPDATA%").is_some());
        assert!(bad(r"%SystemDrive%\").is_some());
        assert!(bad(r"%USERPROFILE%\Documents\Cache").is_some());
        assert!(bad(r"%LOCALAPPDATA%\Temp").is_none());
        assert!(bad(r"%SystemRoot%\Temp").is_none());
    }
}
