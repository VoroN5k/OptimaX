//! Штатні засоби Windows для того, що вручну видаляти не можна: DISM, powercfg, а також
//! «Показати в Провіднику». Запускаються лише системні бінарники з `%SystemRoot%` з фіксованими
//! аргументами; довільних команд немає.

use crate::CleanupError;

/// Вивід штатного засобу.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// Код завершення.
    pub code: i32,
    /// Текст виводу (останні рядки).
    pub output: String,
}

impl ToolOutput {
    /// Успіх (код 0).
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Штатні засоби.
pub trait SystemTools: Send + Sync {
    /// `Dism.exe /Online /Cleanup-Image /StartComponentCleanup` (без `/ResetBase`). Блокує на хвилини.
    fn dism_component_cleanup(&self) -> Result<ToolOutput, CleanupError>;
    /// `powercfg.exe /hibernate on|off`.
    fn set_hibernate(&self, on: bool) -> Result<ToolOutput, CleanupError>;
    /// Відкриває Провідник із виділеним об'єктом (шлях — нормалізований).
    fn reveal(&self, path: &str) -> Result<(), CleanupError>;
}

/// Скільки символів виводу зберігати.
const OUTPUT_TAIL: usize = 4000;

fn tail(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let s = s.trim();
    let start = s
        .char_indices()
        .rev()
        .nth(OUTPUT_TAIL)
        .map_or(0, |(i, _)| i);
    s[start..].to_string()
}

#[cfg(windows)]
pub use self::windows::WinTools;

#[cfg(windows)]
mod windows {
    use super::{tail, SystemTools, ToolOutput};
    use crate::{path, CleanupError};
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Справжні засоби Windows.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct WinTools;

    fn system_root() -> Result<String, CleanupError> {
        let root = std::env::var("SystemRoot")
            .map_err(|_| CleanupError::Tool("немає змінної SystemRoot".into()))?;
        path::normalize(&root).map_err(|e| CleanupError::Tool(e.to_string()))
    }

    /// Шлях до бінарника в `%SystemRoot%\System32`, перевірений на справжність розташування.
    fn system32_exe(name: &str) -> Result<String, CleanupError> {
        let dir = path::join(&system_root()?, "System32");
        let exe = path::join(&dir, name);
        let canonical =
            std::fs::canonicalize(&exe).map_err(|e| CleanupError::Tool(format!("{exe}: {e}")))?;
        let canonical = canonical
            .to_str()
            .and_then(|s| path::normalize(s).ok())
            .ok_or_else(|| CleanupError::Tool(format!("{exe}: неочікуваний шлях")))?;
        if !path::eq_ci(&canonical, &exe) {
            return Err(CleanupError::Tool(format!("{exe}: шлях перенаправлено")));
        }
        Ok(exe)
    }

    fn run(exe: &str, args: &[&str]) -> Result<ToolOutput, CleanupError> {
        tracing::info!("запуск {exe} {}", args.join(" "));
        let out = Command::new(exe)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| CleanupError::Tool(format!("{exe}: {e}")))?;
        let mut text = tail(&out.stdout);
        let err = tail(&out.stderr);
        if !err.is_empty() {
            text.push('\n');
            text.push_str(&err);
        }
        Ok(ToolOutput {
            code: out.status.code().unwrap_or(-1),
            output: text,
        })
    }

    impl SystemTools for WinTools {
        fn dism_component_cleanup(&self) -> Result<ToolOutput, CleanupError> {
            run(
                &system32_exe("Dism.exe")?,
                &["/Online", "/Cleanup-Image", "/StartComponentCleanup"],
            )
        }

        fn set_hibernate(&self, on: bool) -> Result<ToolOutput, CleanupError> {
            run(
                &system32_exe("powercfg.exe")?,
                &["/hibernate", if on { "on" } else { "off" }],
            )
        }

        fn reveal(&self, p: &str) -> Result<(), CleanupError> {
            // Нормалізований шлях не містить лапок, тож безпечно взяти його в лапки самим.
            let p = path::normalize(p).map_err(|e| CleanupError::Tool(e.to_string()))?;
            let explorer = path::join(&system_root()?, "explorer.exe");
            Command::new(explorer)
                .raw_arg(format!("/select,\"{p}\""))
                .spawn()
                .map(|_| ())
                .map_err(|e| CleanupError::Tool(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_end() {
        let long = "а".repeat(OUTPUT_TAIL + 10) + "кінець";
        let t = tail(long.as_bytes());
        assert!(t.ends_with("кінець"));
        assert!(t.chars().count() <= OUTPUT_TAIL + 1);
    }
}
