//! Автозапуск програм: увімкнення й вимкнення так само, як у Диспетчері задач.
//!
//! Стан зберігається в `…\Explorer\StartupApproved\<джерело>` значенням `REG_BINARY`, ім'я якого
//! збігається з іменем запису в Run (або з іменем файлу в теці «Автозавантаження»). Сам запис
//! у Run і ярлик у теці **не змінюються й не видаляються** — лише цей прапорець.
//!
//! Формат офіційно не документований; перевірено на Windows 11 Pro 26300 (2026-10-30) за
//! записами, які зробив Диспетчер задач, і за тим, що реально запущено:
//! - `02 00 00 00` + 8 нульових байтів — увімкнено;
//! - `03 00 00 00` + FILETIME моменту вимкнення — вимкнено (такі програми не запущені);
//! - `04…`, `06…` (зустрічаються в HKLM) — програми запущені, тобто увімкнено;
//! - значення немає — увімкнено.
//!
//! Тому «вимкнено» = непарний перший байт, решта — «увімкнено». Пишемо лише `02` / `03`.

use optimax_core::registry::{RegHive, RegValue};
use serde::{Deserialize, Serialize};

const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_WOW64: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
/// Перший байт: увімкнено.
pub const ENABLED_BYTE: u8 = 0x02;
/// Перший байт: вимкнено.
pub const DISABLED_BYTE: u8 = 0x03;
/// Різниця між епохою FILETIME (1601-01-01) і Unix, с.
const FILETIME_UNIX_DIFF_SECS: u64 = 11_644_473_600;

/// Звідки запускається програма.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StartupSource {
    /// `HKCU\…\Run` — для поточного користувача.
    UserRun,
    /// `HKLM\…\Run` — для всіх користувачів.
    MachineRun,
    /// `HKLM\…\WOW6432Node\…\Run` — 32-бітні програми для всіх користувачів.
    MachineRun32,
    /// Тека «Автозавантаження» поточного користувача.
    UserFolder,
    /// Спільна тека «Автозавантаження».
    CommonFolder,
}

impl StartupSource {
    /// Усі джерела.
    pub const ALL: [StartupSource; 5] = [
        Self::UserRun,
        Self::MachineRun,
        Self::MachineRun32,
        Self::UserFolder,
        Self::CommonFolder,
    ];

    /// Гілка реєстру (і для Run, і для StartupApproved).
    pub fn hive(self) -> RegHive {
        match self {
            Self::UserRun | Self::UserFolder => RegHive::CurrentUser,
            Self::MachineRun | Self::MachineRun32 | Self::CommonFolder => RegHive::LocalMachine,
        }
    }

    /// Ключ Run (`None` — джерело-тека).
    pub fn run_key(self) -> Option<&'static str> {
        match self {
            Self::UserRun | Self::MachineRun => Some(RUN),
            Self::MachineRun32 => Some(RUN_WOW64),
            Self::UserFolder | Self::CommonFolder => None,
        }
    }

    /// Ключ із прапорцями «увімкнено / вимкнено».
    pub fn approved_key(self) -> String {
        let sub = match self {
            Self::UserRun | Self::MachineRun => "Run",
            Self::MachineRun32 => "Run32",
            Self::UserFolder | Self::CommonFolder => "StartupFolder",
        };
        format!(r"{APPROVED}\{sub}")
    }
}

/// Стан за значенням зі StartupApproved. `Err(тип)` — значення не `REG_BINARY`.
pub fn is_enabled(value: Option<&RegValue>) -> Result<bool, &'static str> {
    match value {
        None => Ok(true),
        Some(RegValue::Binary(b)) => Ok(b.first().is_none_or(|first| first & 1 == 0)),
        Some(other) => Err(other.type_name()),
    }
}

/// Значення, яке записує Диспетчер задач: 12 байт, для «вимкнено» — з часом вимкнення.
pub fn approved_value(enabled: bool, now_filetime: u64) -> RegValue {
    let mut b = vec![0u8; 12];
    if enabled {
        b[0] = ENABLED_BYTE;
    } else {
        b[0] = DISABLED_BYTE;
        b[4..].copy_from_slice(&now_filetime.to_le_bytes());
    }
    RegValue::Binary(b)
}

/// Поточний час у форматі FILETIME.
pub fn filetime_now() -> u64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (d.as_secs() + FILETIME_UNIX_DIFF_SECS) * 10_000_000 + u64::from(d.subsec_nanos() / 100)
}

/// Шлях до exe з командного рядка запису Run: у лапках — до закривальної лапки; без лапок
/// (буває `C:\Program Files (x86)\X\x.exe -flag`) — до кінця першого `.exe`, інакше до пробілу.
pub fn exe_from_command(command: &str) -> String {
    let c = command.trim();
    if let Some(rest) = c.strip_prefix('"') {
        return rest.split('"').next().unwrap_or_default().to_string();
    }
    if let Some(i) = c.to_ascii_lowercase().find(".exe") {
        return c[..i + 4].to_string();
    }
    c.split(' ').next().unwrap_or_default().to_string()
}

/// Чи exe лежить у теці Windows (системні компоненти й помічники драйверів у DriverStore).
/// Такі записи показуємо, але вимикати не даємо.
pub fn is_system_path(exe: &str, system_root: &str) -> bool {
    let root = system_root.trim_end_matches('\\').to_ascii_lowercase();
    if root.is_empty() {
        return false;
    }
    let exe = exe.replace('/', "\\").to_ascii_lowercase();
    exe.strip_prefix(&root)
        .is_some_and(|rest| rest.starts_with('\\'))
}

/// Ім'я запису з теки «Автозавантаження» має бути саме ім'ям файлу, без шляху.
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['\\', '/', ':']) && name != "." && name != ".."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_state_matches_task_manager_values() {
        let v = |bytes: &[u8]| RegValue::Binary(bytes.to_vec());
        assert_eq!(is_enabled(None), Ok(true));
        assert_eq!(is_enabled(Some(&v(&[2, 0, 0, 0]))), Ok(true));
        assert_eq!(
            is_enabled(Some(&v(&[
                3, 0, 0, 0, 0x2F, 0xBA, 0x7D, 0x4C, 0xE8, 0xB5, 0xDA, 1
            ]))),
            Ok(false)
        );
        assert_eq!(is_enabled(Some(&v(&[4, 0, 0, 0]))), Ok(true));
        assert_eq!(is_enabled(Some(&v(&[6, 0, 0, 0]))), Ok(true));
        assert_eq!(is_enabled(Some(&v(&[]))), Ok(true));
        assert_eq!(is_enabled(Some(&RegValue::Dword(3))), Err("REG_DWORD"));
    }

    #[test]
    fn written_values_have_task_manager_layout() {
        assert_eq!(
            approved_value(true, 123),
            RegValue::Binary(vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        let ft = 0x01DA_B5E8_4C7D_BA2F_u64;
        assert_eq!(
            approved_value(false, ft),
            RegValue::Binary(vec![
                3, 0, 0, 0, 0x2F, 0xBA, 0x7D, 0x4C, 0xE8, 0xB5, 0xDA, 1
            ])
        );
        assert_eq!(is_enabled(Some(&approved_value(false, ft))), Ok(false));
        assert_eq!(is_enabled(Some(&approved_value(true, 0))), Ok(true));
        // FILETIME зараз — після 2020 року.
        assert!(filetime_now() > 132_223_104_000_000_000);
    }

    #[test]
    fn exe_paths_from_real_run_commands() {
        assert_eq!(
            exe_from_command(
                r#""C:\Users\u\AppData\Local\Discord\Update.exe" --processStart Discord.exe"#
            ),
            r"C:\Users\u\AppData\Local\Discord\Update.exe"
        );
        assert_eq!(
            exe_from_command(
                r"C:\Program Files (x86)\Overwolf\OverwolfLauncher.exe -overwolfsilent"
            ),
            r"C:\Program Files (x86)\Overwolf\OverwolfLauncher.exe"
        );
        assert_eq!(
            exe_from_command(r"C:\WINDOWS\system32\SecurityHealthSystray.exe"),
            r"C:\WINDOWS\system32\SecurityHealthSystray.exe"
        );
        assert_eq!(exe_from_command("tool -x"), "tool");
        assert_eq!(exe_from_command(""), "");
    }

    #[test]
    fn system_paths_are_detected() {
        let root = r"C:\WINDOWS";
        assert!(is_system_path(
            r"C:\WINDOWS\system32\SecurityHealthSystray.exe",
            root
        ));
        assert!(is_system_path(
            r"C:\WINDOWS\System32\DriverStore\FileRepository\x\RtkAudUService64.exe",
            r"C:\Windows\"
        ));
        assert!(!is_system_path(r"C:\Windows.old\x.exe", root));
        assert!(!is_system_path(r"C:\Program Files\LGHUB\lghub.exe", root));
        assert!(!is_system_path(r"C:\WINDOWS\x.exe", ""));
    }

    #[test]
    fn sources_map_to_task_manager_keys() {
        assert_eq!(
            StartupSource::MachineRun32.approved_key(),
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32"
        );
        assert_eq!(StartupSource::MachineRun32.run_key(), Some(RUN_WOW64));
        assert_eq!(StartupSource::CommonFolder.hive(), RegHive::LocalMachine);
        assert_eq!(StartupSource::UserFolder.run_key(), None);
        assert!(is_plain_file_name("Tailscale.lnk"));
        assert!(!is_plain_file_name(r"..\x.lnk"));
        assert!(!is_plain_file_name(".."));
    }
}
