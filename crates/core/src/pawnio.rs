//! Драйвер PawnIO (pawnio.eu): доступ до датчиків процесора, які Windows без драйвера ядра не
//! віддає (температура Ryzen, частоти шини пам'яті).
//!
//! Програма драйвер **не встановлює й не завантажує з мережі**: користувач ставить його сам, а
//! вмикає читання явною згодою в налаштуваннях. PawnIO виконує лише модулі, підписані автором
//! драйвера; модулі вбудовано в програму (крейт `optimax-metrics`), їхній SHA-256 перевіряється
//! перед кожним завантаженням.
//!
//! Обмін — `DeviceIoControl` з пристроєм `\\?\GLOBALROOT\Device\PawnIO` (як у LibreHardwareMonitor):
//! `LOAD_BINARY` — підписаний модуль, `EXECUTE_FN` — виклик функції модуля: 32 байти ASCII-імені
//! (до 31 символу, решта нулі), далі аргументи `i64` (little-endian); відповідь — масив `i64`.
//! Ліцензія драйвера (GPL-2.0) містить виняток для програм, що спілкуються з ним лише так.

use crate::error::CoreError;
use crate::registry::{RegHive, RegistryApi};

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;
#[cfg(windows)]
pub use self::windows::{Module, PciLock};

/// Ключ реєстру, за яким видно встановлений PawnIO.
pub const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO";
/// Довжина поля імені функції у вхідному буфері.
pub const FN_NAME_LEN: usize = 32;

/// Версія встановленого PawnIO (`None` — не встановлено).
pub fn installed_version(reg: &dyn RegistryApi) -> Option<String> {
    reg.read_value(RegHive::LocalMachine, UNINSTALL_KEY, "DisplayVersion")
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|v| !v.trim().is_empty())
}

/// Вхідний буфер виклику функції модуля.
pub fn exec_input(name: &str, args: &[i64]) -> Result<Vec<u8>, CoreError> {
    if name.is_empty() || name.len() >= FN_NAME_LEN || !name.is_ascii() {
        return Err(CoreError::InvalidData(format!(
            "ім'я функції PawnIO: {name}"
        )));
    }
    let mut buf = vec![0u8; FN_NAME_LEN + args.len() * 8];
    buf[..name.len()].copy_from_slice(name.as_bytes());
    for (i, a) in args.iter().enumerate() {
        let at = FN_NAME_LEN + i * 8;
        buf[at..at + 8].copy_from_slice(&a.to_le_bytes());
    }
    Ok(buf)
}

/// Відповідь функції модуля: `i64` у little-endian (неповний хвіст відкидається).
pub fn exec_output(bytes: &[u8]) -> Vec<i64> {
    bytes
        .chunks_exact(8)
        .map(|c| {
            let mut a = [0u8; 8];
            a.copy_from_slice(c);
            i64::from_le_bytes(a)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{MemRegistry, RegValue};

    #[test]
    fn input_is_name_padded_to_32_bytes_then_le_args() {
        let b = exec_input("ioctl_read_smn", &[0x59800, -1]).expect("буфер");
        assert_eq!(b.len(), 32 + 16);
        assert_eq!(&b[..14], b"ioctl_read_smn");
        assert!(b[14..32].iter().all(|&x| x == 0));
        assert_eq!(&b[32..40], &0x59800i64.to_le_bytes());
        assert_eq!(&b[40..48], &(-1i64).to_le_bytes());
        assert_eq!(exec_output(&b[32..]), vec![0x59800, -1]);
    }

    #[test]
    fn bad_names_are_rejected() {
        assert!(exec_input("", &[]).is_err());
        assert!(exec_input(&"x".repeat(32), &[]).is_err());
        assert!(exec_input("функція", &[]).is_err());
        assert!(exec_input(&"x".repeat(31), &[]).is_ok());
    }

    #[test]
    fn version_comes_from_uninstall_key() {
        let mut reg = MemRegistry::new();
        assert_eq!(installed_version(&reg), None);
        reg.create_key(RegHive::LocalMachine, UNINSTALL_KEY);
        reg.set(
            RegHive::LocalMachine,
            UNINSTALL_KEY,
            "DisplayVersion",
            RegValue::String("2.2.0.0".into()),
        );
        assert_eq!(installed_version(&reg).as_deref(), Some("2.2.0.0"));
    }
}
