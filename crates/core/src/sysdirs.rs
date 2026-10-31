//! Системні теки Windows — з API, а не зі змінних середовища.
//!
//! Змінні середовища (`%SystemRoot%`, `%ProgramFiles%`…) процес успадковує від того, хто його
//! запустив, і частину з них користувач може перевизначити (`HKCU\Environment`). Програмі з
//! правами адміністратора не можна будувати за ними ні шлях до системного exe чи DLL, ні
//! чорний список очищення.
//!
//! - [`windows_dir`], [`system32`] — від ядра (`GetSystemWindowsDirectoryW`,
//!   `GetSystemDirectoryW`).
//! - [`known`] — `SHGetKnownFolderPath`. Системні теки (`ProgramFiles`, `ProgramData`, профіль,
//!   `Public`) задає HKLM; теки користувача (`LocalAppData`, `RoamingAppData`, `OneDrive`,
//!   «Автозавантаження») — `HKCU\…\User Shell Folders`, тобто їх може змінити сам користувач.
//!   Тому теки користувача не годяться як межа безпеки — лише як місце, де шукати його файли.

use std::path::PathBuf;

#[cfg(windows)]
mod windows;

/// Відома тека.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folder {
    /// `C:\ProgramData` (HKLM).
    ProgramData,
    /// `C:\Program Files` для цього процесу (64-бітного) (HKLM).
    ProgramFiles,
    /// `C:\Program Files` — 64-бітна (HKLM).
    ProgramFilesX64,
    /// `C:\Program Files (x86)` (HKLM).
    ProgramFilesX86,
    /// Профіль користувача `C:\Users\<ім'я>` (HKLM `ProfileList`).
    Profile,
    /// `C:\Users\Public` (HKLM).
    Public,
    /// `%LOCALAPPDATA%` (задає користувач).
    LocalAppData,
    /// `%APPDATA%` (задає користувач).
    RoamingAppData,
    /// Тека OneDrive (задає користувач; може не існувати).
    OneDrive,
    /// «Автозавантаження» користувача (задає користувач).
    Startup,
    /// Спільне «Автозавантаження» (HKLM).
    CommonStartup,
}

/// Тека Windows (`C:\Windows`).
pub fn windows_dir() -> Option<PathBuf> {
    imp::windows_dir()
}

/// `C:\Windows\System32`.
pub fn system32() -> Option<PathBuf> {
    imp::system32()
}

/// Системний диск у вигляді `C:`.
pub fn system_drive() -> Option<String> {
    let w = windows_dir()?;
    let s = w.to_str()?;
    let b = s.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':').then(|| s[..2].to_string())
}

/// Відома тека; `None`, якщо її немає в цій системі чи для цього користувача.
pub fn known(folder: Folder) -> Option<PathBuf> {
    imp::known(folder)
}

/// `explorer.exe` з теки Windows (не з `PATH`).
pub fn explorer() -> Option<PathBuf> {
    windows_dir().map(|w| w.join("explorer.exe"))
}

#[cfg(windows)]
use self::windows as imp;

#[cfg(not(windows))]
mod imp {
    use super::Folder;
    use std::path::PathBuf;

    pub(super) fn windows_dir() -> Option<PathBuf> {
        None
    }

    pub(super) fn system32() -> Option<PathBuf> {
        None
    }

    pub(super) fn known(_folder: Folder) -> Option<PathBuf> {
        None
    }
}
