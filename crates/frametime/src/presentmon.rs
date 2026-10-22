//! Вбудований PresentMon: розпакування, перевірка SHA-256 і запуск.
//!
//! Правила безпеки (CLAUDE.md, розд. 2.1 і 5.7):
//! - бінарник вшитий у програму; з мережі нічого не завантажується;
//! - SHA-256 перевіряється при збірці (`build.rs`), перед розпакуванням і **перед кожним
//!   запуском**; не збігся — запуску немає;
//! - під час перевірки й запуску файл відкрито без дозволу на запис/видалення іншим процесам,
//!   щоб його не можна було підмінити між перевіркою та стартом;
//! - каталог `%LOCALAPPDATA%\OptimaX\bin` отримує права «запис лише для адміністраторів».

use crate::FrametimeError;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Версія вбудованого PresentMon.
pub const PRESENTMON_VERSION: &str = "2.6.0";
/// Ім'я файлу після розпакування.
pub const PRESENTMON_FILE_NAME: &str = "PresentMon-2.6.0-x64.exe";
/// Очікуваний SHA-256. Має збігатися з `presentmon.sha256` (перевіряє тест і build.rs).
pub const PRESENTMON_SHA256: &str =
    "b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af";
/// Ім'я ETW-сесії, щоб не заважати іншим копіям PresentMon (RTSS, FrameView тощо).
pub const SESSION_NAME: &str = "OptimaX";

const EMBEDDED: &[u8] = include_bytes!("../bin/PresentMon-2.6.0-x64.exe");

/// Найдовший дозволений запис.
pub const MAX_CAPTURE: Duration = Duration::from_secs(600);

/// SHA-256 у нижньому регістрі hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn verify_bytes(bytes: &[u8]) -> Result<(), FrametimeError> {
    let actual = sha256_hex(bytes);
    if actual == PRESENTMON_SHA256 {
        Ok(())
    } else {
        Err(FrametimeError::HashMismatch {
            expected: PRESENTMON_SHA256.into(),
            actual,
        })
    }
}

/// Що записувати.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Target {
    /// За іменем exe, напр. `GeometryDash.exe`.
    ProcessName(String),
    /// За PID.
    ProcessId(u32),
}

impl Target {
    /// Перевіряє ім'я процесу: лише ім'я файлу, без шляхів і лапок.
    pub fn validate(&self) -> Result<(), FrametimeError> {
        match self {
            Self::ProcessId(0) => Err(FrametimeError::InvalidTarget("PID 0".into())),
            Self::ProcessId(_) => Ok(()),
            Self::ProcessName(name) => {
                let ok = !name.trim().is_empty()
                    && name.len() <= 260
                    && !name
                        .chars()
                        .any(|c| matches!(c, '\\' | '/' | '"' | ':' | '\0'));
                if ok {
                    Ok(())
                } else {
                    Err(FrametimeError::InvalidTarget(name.clone()))
                }
            }
        }
    }
}

/// Аргументи командного рядка для запису (чиста функція — легко перевірити тестом).
pub fn capture_args(target: &Target, duration: Duration) -> Vec<String> {
    let secs = duration
        .clamp(Duration::from_secs(1), MAX_CAPTURE)
        .as_secs();
    let mut args: Vec<String> = match target {
        Target::ProcessName(n) => vec!["--process_name".into(), n.clone()],
        Target::ProcessId(p) => vec!["--process_id".into(), p.to_string()],
    };
    args.extend(
        [
            "--output_stdout",
            "--no_console_stats",
            "--qpc_time",
            "--session_name",
            SESSION_NAME,
            "--stop_existing_session",
            "--terminate_on_proc_exit",
            "--timed",
        ]
        .map(String::from),
    );
    args.push(secs.to_string());
    args.push("--terminate_after_timed".into());
    args
}

/// Аргументи для зупинки нашої ETW-сесії.
pub fn stop_args() -> Vec<String> {
    vec![
        "--session_name".into(),
        SESSION_NAME.into(),
        "--terminate_existing_session".into(),
    ]
}

/// Каталог за замовчуванням: `%LOCALAPPDATA%\OptimaX\bin`.
pub fn default_install_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("OptimaX").join("bin"))
}

/// Відкриває файл так, щоб інші процеси не могли його змінити, перейменувати чи видалити,
/// доки дескриптор відкритий.
fn open_locked(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        opts.share_mode(FILE_SHARE_READ);
    }
    opts.open(path)
}

/// Розпакований і перевірений PresentMon.
#[derive(Debug, Clone)]
pub struct BundledPresentMon {
    path: PathBuf,
    hardened: bool,
}

impl BundledPresentMon {
    /// Розпаковує вбудований PresentMon у `dir` (якщо там ще немає правильної копії).
    pub fn install(dir: &Path) -> Result<Self, FrametimeError> {
        // Вшита копія теж перевіряється: build.rs міг бути обійдений.
        verify_bytes(EMBEDDED)?;
        fs::create_dir_all(dir)?;
        let hardened = harden_dir(dir);
        let path = dir.join(PRESENTMON_FILE_NAME);

        let up_to_date = match fs::read(&path) {
            Ok(existing) => verify_bytes(&existing).is_ok(),
            Err(_) => false,
        };
        if !up_to_date {
            // Пишемо у тимчасовий файл і лише після перевірки замінюємо основний.
            let tmp = dir.join(format!("{PRESENTMON_FILE_NAME}.tmp"));
            {
                let mut f = File::create(&tmp)?;
                f.write_all(EMBEDDED)?;
                f.sync_all()?;
            }
            verify_bytes(&fs::read(&tmp)?)?;
            if path.exists() {
                fs::remove_file(&path)?;
            }
            fs::rename(&tmp, &path)?;
            tracing::info!(
                "PresentMon {PRESENTMON_VERSION} розпаковано: {}",
                path.display()
            );
        }
        Ok(Self { path, hardened })
    }

    /// Розпаковує в каталог за замовчуванням.
    pub fn install_default() -> Result<Self, FrametimeError> {
        let dir = default_install_dir()
            .ok_or_else(|| FrametimeError::InvalidTarget("немає змінної LOCALAPPDATA".into()))?;
        Self::install(&dir)
    }

    /// Шлях до exe.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Чи вдалося обмежити права на каталог (потрібні права адміністратора).
    pub fn is_dir_hardened(&self) -> bool {
        self.hardened
    }

    /// Перевіряє хеш файлу на диску й повертає заблокований дескриптор.
    fn verified_handle(&self) -> Result<File, FrametimeError> {
        let mut f = open_locked(&self.path)?;
        let mut bytes = Vec::with_capacity(EMBEDDED.len());
        f.read_to_end(&mut bytes)?;
        if let Err(e) = verify_bytes(&bytes) {
            tracing::error!("PresentMon не пройшов перевірку перед запуском: {e}");
            return Err(e);
        }
        Ok(f)
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.path);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    /// Запускає запис. Хеш перевіряється безпосередньо перед стартом.
    pub fn spawn_capture(
        &self,
        target: &Target,
        duration: Duration,
    ) -> Result<Child, FrametimeError> {
        target.validate()?;
        let lock = self.verified_handle()?;
        let child = self.command(&capture_args(target, duration)).spawn()?;
        // Образ уже завантажено системою — далі файл змінити неможливо.
        drop(lock);
        Ok(child)
    }

    /// Зупиняє нашу ETW-сесію (якщо вона лишилася після збою). Хеш перевіряється так само.
    pub fn stop_session(&self) -> Result<(), FrametimeError> {
        let lock = self.verified_handle()?;
        let mut child = self.command(&stop_args()).spawn()?;
        drop(lock);
        child.wait()?;
        Ok(())
    }
}

#[cfg(windows)]
fn harden_dir(dir: &Path) -> bool {
    match crate::acl::restrict_to_admins(dir) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(
                "не вдалося обмежити права на {} ({e}); захист тримається на перевірці SHA-256",
                dir.display()
            );
            false
        }
    }
}

#[cfg(not(windows))]
fn harden_dir(_dir: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_binary_matches_constant_and_hash_file() {
        assert_eq!(sha256_hex(EMBEDDED), PRESENTMON_SHA256);
        let file = include_str!("../presentmon.sha256");
        let mut parts = file.split_whitespace();
        assert_eq!(parts.next(), Some(PRESENTMON_SHA256));
        assert_eq!(parts.next(), Some(PRESENTMON_FILE_NAME));
    }

    #[test]
    fn tampered_bytes_are_rejected() {
        let mut bad = EMBEDDED.to_vec();
        bad[1000] ^= 0xFF;
        assert!(matches!(
            verify_bytes(&bad),
            Err(FrametimeError::HashMismatch { .. })
        ));
    }

    #[test]
    fn capture_args_are_fixed_and_bounded() {
        let a = capture_args(
            &Target::ProcessName("game.exe".into()),
            Duration::from_secs(30),
        );
        assert_eq!(&a[..2], &["--process_name", "game.exe"]);
        assert!(a.contains(&"--qpc_time".to_string()));
        assert!(a.contains(&"--output_stdout".to_string()));
        let timed = a.iter().position(|s| s == "--timed").unwrap();
        assert_eq!(a[timed + 1], "30");
        let a = capture_args(&Target::ProcessId(42), Duration::from_secs(100_000));
        assert_eq!(&a[..2], &["--process_id", "42"]);
        let timed = a.iter().position(|s| s == "--timed").unwrap();
        assert_eq!(a[timed + 1], "600");
    }

    #[test]
    fn target_validation() {
        assert!(Target::ProcessName("GeometryDash.exe".into())
            .validate()
            .is_ok());
        assert!(Target::ProcessName(r"..\evil.exe".into())
            .validate()
            .is_err());
        assert!(Target::ProcessName("a\" --etl_file x".into())
            .validate()
            .is_err());
        assert!(Target::ProcessName("  ".into()).validate().is_err());
        assert!(Target::ProcessId(0).validate().is_err());
    }

    #[test]
    fn install_writes_verified_copy_and_repairs_tampering() {
        let dir = std::env::temp_dir().join(format!("optimax-pm-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pm = BundledPresentMon::install(&dir).unwrap();
        assert_eq!(sha256_hex(&fs::read(pm.path()).unwrap()), PRESENTMON_SHA256);
        assert!(pm.verified_handle().is_ok());

        // Підміна файлу: запуск має бути заборонений, а повторне встановлення — відновити копію.
        let hardened = pm.is_dir_hardened();
        if !hardened {
            fs::write(pm.path(), b"not presentmon").unwrap();
            assert!(matches!(
                pm.verified_handle(),
                Err(FrametimeError::HashMismatch { .. })
            ));
            let pm = BundledPresentMon::install(&dir).unwrap();
            assert!(pm.verified_handle().is_ok());
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
