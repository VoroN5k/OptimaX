//! Служба OptimaX (етап 1 з `docs/SERVICE.md`): той самий `optimax.exe` з `--service` працює як
//! служба Windows під SYSTEM і слухає іменований канал. Інтерфейс з'єднується з нею як клієнт.
//!
//! Захист каналу (розділ 6 `docs/SERVICE.md`):
//! - дескриптор безпеки: SYSTEM, Адміністратори й користувач, що встановив службу;
//!   віддалені клієнти відхиляються, канал створює лише служба (перший екземпляр);
//! - служба приймає лише клієнта, чий exe — той самий файл, що й у служби
//!   (`%ProgramFiles%\OptimaX\OptimaX.exe`); клієнт перевіряє, що власник каналу — SYSTEM чи
//!   Адміністратори, а з правами — ще й що на тому кінці цей самий exe;
//! - закритий перелік команд ([`protocol::Command`]).
//!
//! Встановлюється лише за явною дією користувача ([`install`]); автозапуск — службою Windows.

pub mod logfile;
pub mod protocol;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod pipe;

#[cfg(windows)]
pub mod client;
#[cfg(windows)]
pub mod host;
#[cfg(windows)]
pub mod install;

use std::path::{Path, PathBuf};

/// Ім'я exe у теці встановлення (як його кладе інсталятор NSIS).
pub const EXE_NAME: &str = "OptimaX.exe";

/// Помилки служби.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// Помилка Windows (диспетчер служб, файли).
    #[error("{0}")]
    Os(String),
    /// Канал чи протокол.
    #[error(transparent)]
    Protocol(#[from] protocol::ProtocolError),
    /// На іншому кінці каналу не наш exe.
    #[error("на іншому кінці каналу чужа програма: {0}")]
    Untrusted(String),
    /// Версія протоколу не збіглась.
    #[error("служба іншої версії ({0}) — переустановіть її")]
    Incompatible(String),
    /// Служба відповіла помилкою.
    #[error("служба: {0}")]
    Refused(String),
}

impl From<std::io::Error> for ServiceError {
    fn from(e: std::io::Error) -> Self {
        Self::Os(e.to_string())
    }
}

/// Тека встановлення: `%ProgramFiles%\OptimaX` (запис лише для адміністраторів).
pub fn install_dir() -> Option<PathBuf> {
    optimax_core::sysdirs::known(optimax_core::sysdirs::Folder::ProgramFilesX64)
        .or_else(|| optimax_core::sysdirs::known(optimax_core::sysdirs::Folder::ProgramFiles))
        .map(|p| p.join(optimax_core::datadir::APP_DIR))
}

/// Exe, з якого дозволено запускати службу.
pub fn service_exe() -> Option<PathBuf> {
    install_dir().map(|d| d.join(EXE_NAME))
}

/// Той самий файл (без різниці в регістрі й формі шляху). Неіснуючі шляхи — різні.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

/// SID лише з цифр, `S` і дефісів — інакше його не можна підставляти в SDDL.
pub fn valid_sid(sid: &str) -> bool {
    sid.starts_with("S-1-")
        && sid.len() <= 184
        && sid
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
}

/// Дескриптор безпеки каналу: SYSTEM і Адміністратори — повний доступ; користувач `sid` —
/// читання й запис даних (0x12008B: без `FILE_CREATE_PIPE_INSTANCE`, щоб він не міг створювати
/// власні екземпляри каналу з цим ім'ям). Захищений DACL (`P`) — без успадкування.
pub fn pipe_sddl(sid: &str) -> Option<String> {
    valid_sid(sid).then(|| format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12008b;;;{sid})"))
}

/// Аргументи запуску служби в її конфігурації (змінювати їх можуть лише адміністратори).
pub fn service_args(sid: &str) -> Vec<String> {
    vec!["--service".into(), "--user".into(), sid.into()]
}

/// SID із `--user <SID>` серед аргументів процесу.
pub fn user_from_args(args: &[String]) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == "--user")
        .map(|w| w[1].clone())
        .filter(|s| valid_sid(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sid_is_validated_before_going_into_sddl() {
        let sid = "S-1-5-21-1004336348-1177238915-682003330-1001";
        assert!(valid_sid(sid));
        let sddl = pipe_sddl(sid).expect("sddl");
        assert!(sddl.starts_with("D:P("));
        assert!(sddl.ends_with(&format!("(A;;0x12008b;;;{sid})")));
        for bad in [
            "",
            "S-1-5)(A;;GA;;;WD",
            "S-1-5-21-1 ",
            "1-5-21",
            "S-1-5-21-x",
        ] {
            assert!(!valid_sid(bad), "{bad}");
            assert!(pipe_sddl(bad).is_none());
        }
    }

    #[test]
    fn user_comes_from_service_arguments() {
        let sid = "S-1-5-21-1-2-3-1001";
        let args = service_args(sid);
        assert_eq!(args, ["--service", "--user", sid]);
        assert_eq!(user_from_args(&args).as_deref(), Some(sid));
        assert_eq!(user_from_args(&["--service".into()]), None);
        assert_eq!(
            user_from_args(&["--user".into(), "S-1-5)(A;;GA;;;WD)".into()]),
            None
        );
    }

    #[test]
    fn missing_files_are_never_the_same() {
        let p = Path::new(r"C:\definitely\missing\optimax.exe");
        assert!(!same_file(p, p));
    }
}
