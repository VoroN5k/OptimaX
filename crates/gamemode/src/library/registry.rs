//! GOG і Battle.net — з реєстру (звірено 2026-11-18 на реальному ПК).
//!
//! - GOG: `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>` → `gameName`, `path`, `exeFile`.
//! - Battle.net: записи видалення програм із видавцем Blizzard / Activision; сам лаунчер
//!   (`DisplayIcon` → `Battle.net.exe`) пропускається.

use optimax_core::error::CoreError;
use optimax_core::registry::{RegHive, RegValue, RegistryApi};

/// Корінь ігор GOG.
pub const GOG_ROOT: &str = r"SOFTWARE\WOW6432Node\GOG.com\Games";
/// Гілки видалення програм (32- і 64-бітна), де Battle.net реєструє ігри.
pub const UNINSTALL_ROOTS: [&str; 2] = [
    r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
];

/// Гра, знайдена в реєстрі.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegGame {
    /// Стабільний ідентифікатор: `gog:<підключ>` або `bnet:<підключ>`.
    pub key: String,
    /// Назва.
    pub name: String,
    /// Тека встановлення.
    pub install_dir: String,
    /// Ім'я exe з реєстру (лише підказка).
    pub exe_hint: Option<String>,
}

/// Ігри GOG.
pub fn gog_games(reg: &dyn RegistryApi) -> Result<Vec<RegGame>, CoreError> {
    let mut out = Vec::new();
    for sub in reg.subkey_names(RegHive::LocalMachine, GOG_ROOT)? {
        let key = format!(r"{GOG_ROOT}\{sub}");
        let (Some(name), Some(dir)) = (text(reg, &key, "gameName")?, text(reg, &key, "path")?)
        else {
            continue;
        };
        out.push(RegGame {
            key: format!("gog:{sub}"),
            name,
            install_dir: dir,
            exe_hint: text(reg, &key, "exeFile")?,
        });
    }
    Ok(out)
}

/// Ігри Battle.net.
pub fn battlenet_games(reg: &dyn RegistryApi) -> Result<Vec<RegGame>, CoreError> {
    let mut out = Vec::new();
    for root in UNINSTALL_ROOTS {
        for sub in reg.subkey_names(RegHive::LocalMachine, root)? {
            let key = format!(r"{root}\{sub}");
            let publisher = text(reg, &key, "Publisher")?.unwrap_or_default();
            if !(publisher.contains("Blizzard") || publisher.contains("Activision")) {
                continue;
            }
            let (Some(name), Some(dir)) = (
                text(reg, &key, "DisplayName")?,
                text(reg, &key, "InstallLocation")?,
            ) else {
                continue;
            };
            let exe = text(reg, &key, "DisplayIcon")?.and_then(|icon| icon_exe(&icon));
            if exe
                .as_deref()
                .is_some_and(|e| e.eq_ignore_ascii_case("Battle.net.exe"))
            {
                continue;
            }
            out.push(RegGame {
                key: format!("bnet:{sub}"),
                name,
                install_dir: dir,
                exe_hint: exe,
            });
        }
    }
    Ok(out)
}

/// Непорожній рядок значення (`REG_SZ` або `REG_EXPAND_SZ` без розгортання), обрізаний.
fn text(reg: &dyn RegistryApi, key: &str, name: &str) -> Result<Option<String>, CoreError> {
    Ok(match reg.read_value(RegHive::LocalMachine, key, name)? {
        Some(RegValue::String(v) | RegValue::ExpandString(v)) => Some(v),
        _ => None,
    }
    .map(|v| v.trim().to_string())
    .filter(|v| !v.is_empty()))
}

/// Ім'я exe з `DisplayIcon`: без лапок, без індексу іконки `,0`, лише ім'я файлу.
fn icon_exe(icon: &str) -> Option<String> {
    let path = icon.split(',').next()?.trim().trim_matches('"');
    let file = path.rsplit(['\\', '/']).next()?.trim();
    file.to_ascii_lowercase()
        .ends_with(".exe")
        .then(|| file.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::registry::MemRegistry;

    const LM: RegHive = RegHive::LocalMachine;

    fn s(v: &str) -> RegValue {
        RegValue::String(v.into())
    }

    fn gog() -> MemRegistry {
        let mut reg = MemRegistry::new();
        let key = format!(r"{GOG_ROOT}\1429864849");
        reg.set(LM, &key, "gameName", s("Kerbal Space Program"));
        reg.set(LM, &key, "path", s(r"F:\Games\Kerbal Space Program"));
        reg.set(
            LM,
            &key,
            "exe",
            s(r"F:\Games\Kerbal Space Program\KSP_x64.exe"),
        );
        reg.set(LM, &key, "exeFile", s("KSP_x64.exe"));
        reg
    }

    fn uninstall(reg: &mut MemRegistry, root: &str, sub: &str, vals: &[(&str, &str)]) {
        let key = format!(r"{root}\{sub}");
        for (n, v) in vals {
            reg.set(LM, &key, n, s(v));
        }
    }

    #[test]
    fn gog_reads_game() {
        let games = gog_games(&gog()).unwrap();
        assert_eq!(games.len(), 1);
        let g = &games[0];
        assert_eq!(g.name, "Kerbal Space Program");
        assert_eq!(g.install_dir, r"F:\Games\Kerbal Space Program");
        assert_eq!(g.exe_hint.as_deref(), Some("KSP_x64.exe"));
        assert!(g.key.eq_ignore_ascii_case("gog:1429864849"), "{}", g.key);
    }

    #[test]
    fn gog_missing_root_is_empty() {
        assert!(gog_games(&MemRegistry::new()).unwrap().is_empty());
    }

    #[test]
    fn gog_without_path_is_skipped() {
        let mut reg = MemRegistry::new();
        reg.set(LM, &format!(r"{GOG_ROOT}\1"), "gameName", s("X"));
        assert!(gog_games(&reg).unwrap().is_empty());
    }

    #[test]
    fn bnet_reads_blizzard_game() {
        let mut reg = MemRegistry::new();
        uninstall(
            &mut reg,
            UNINSTALL_ROOTS[0],
            "Call of Duty",
            &[
                ("DisplayName", "Call of Duty"),
                ("Publisher", "Activision"),
                ("InstallLocation", r"F:\Call of Duty"),
                ("DisplayIcon", r"F:\Call of Duty\_retail_\cod.exe"),
            ],
        );
        let games = battlenet_games(&reg).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Call of Duty");
        assert_eq!(games[0].install_dir, r"F:\Call of Duty");
        assert_eq!(games[0].exe_hint.as_deref(), Some("cod.exe"));
        assert!(games[0].key.starts_with("bnet:"));
    }

    #[test]
    fn bnet_reads_both_roots() {
        let mut reg = MemRegistry::new();
        for (i, root) in UNINSTALL_ROOTS.iter().enumerate() {
            uninstall(
                &mut reg,
                root,
                &format!("G{i}"),
                &[
                    ("DisplayName", &format!("Game {i}")),
                    ("Publisher", "Blizzard Entertainment"),
                    ("InstallLocation", &format!(r"F:\G{i}")),
                ],
            );
        }
        assert_eq!(battlenet_games(&reg).unwrap().len(), 2);
    }

    #[test]
    fn bnet_skips_launcher() {
        let mut reg = MemRegistry::new();
        uninstall(
            &mut reg,
            UNINSTALL_ROOTS[0],
            "Battle.net",
            &[
                ("DisplayName", "Battle.net"),
                ("Publisher", "Blizzard Entertainment"),
                ("InstallLocation", r"E:\Battle.net"),
                ("DisplayIcon", r"E:\Battle.net\Battle.net.exe"),
            ],
        );
        assert!(battlenet_games(&reg).unwrap().is_empty());
    }

    #[test]
    fn bnet_skips_other_publishers() {
        let mut reg = MemRegistry::new();
        uninstall(
            &mut reg,
            UNINSTALL_ROOTS[1],
            "Steam",
            &[
                ("DisplayName", "Steam"),
                ("Publisher", "Valve Corporation"),
                ("InstallLocation", r"D:\Steam"),
            ],
        );
        assert!(battlenet_games(&reg).unwrap().is_empty());
    }

    #[test]
    fn bnet_skips_empty_install_location() {
        let mut reg = MemRegistry::new();
        uninstall(
            &mut reg,
            UNINSTALL_ROOTS[0],
            "X",
            &[("DisplayName", "X"), ("Publisher", "Activision")],
        );
        assert!(battlenet_games(&reg).unwrap().is_empty());
    }

    #[test]
    fn bnet_strips_icon_index() {
        let mut reg = MemRegistry::new();
        uninstall(
            &mut reg,
            UNINSTALL_ROOTS[0],
            "Cod",
            &[
                ("DisplayName", "Call of Duty"),
                ("Publisher", "Activision"),
                ("InstallLocation", r"F:\Call of Duty"),
                ("DisplayIcon", r#""F:\Call of Duty\_retail_\cod.exe",0"#),
            ],
        );
        assert_eq!(
            battlenet_games(&reg).unwrap()[0].exe_hint.as_deref(),
            Some("cod.exe")
        );
    }
}
