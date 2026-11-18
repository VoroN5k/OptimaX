//! Epic Games Launcher: `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item` (JSON;
//! формат звірено 2026-11-18 на реальному ПК).
//!
//! `LaunchExecutable` часто вказує на проміжний лаунчер (`PlayRDR2.exe`, `launcher_epic.exe`),
//! тому підказкою exe служить лише `MainWindowProcessName`, і то без підтвердження за текою.

use super::{norm_dir, LibraryError};
use serde::Deserialize;

/// Гра з маніфесту Epic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpicGame {
    /// Внутрішнє ім'я застосунку (`AppName`), стабільне між запусками.
    pub app_name: String,
    /// Назва (`DisplayName`).
    pub name: String,
    /// Тека встановлення (`InstallLocation`).
    pub install_dir: String,
    /// `MainWindowProcessName`, якщо не порожній.
    pub exe_hint: Option<String>,
}

/// Розбирає один `.item`. `Ok(None)` — не гра (рушій, плагін) або встановлення не завершене.
pub fn parse_item(text: &str) -> Result<Option<EpicGame>, LibraryError> {
    let item: Item =
        serde_json::from_str(text).map_err(|e| LibraryError::parse("Epic .item", e.to_string()))?;
    let is_game = item
        .app_categories
        .iter()
        .any(|c| c.eq_ignore_ascii_case("games"));
    let name = item.display_name.trim();
    let dir = item.install_location.trim();
    if !is_game || item.incomplete || name.is_empty() || dir.is_empty() {
        return Ok(None);
    }
    let hint = item.main_window_process_name.trim();
    Ok(Some(EpicGame {
        app_name: item.app_name.trim().to_string(),
        name: name.to_string(),
        install_dir: dir.to_string(),
        exe_hint: (!hint.is_empty()).then(|| hint.to_string()),
    }))
}

/// Прибирає дублікати: той самий `app_name` або та сама тека (без урахування регістру).
pub fn dedup(games: Vec<EpicGame>) -> Vec<EpicGame> {
    let mut out: Vec<EpicGame> = Vec::with_capacity(games.len());
    for g in games {
        let dup = out.iter().any(|o| {
            (!g.app_name.is_empty() && o.app_name == g.app_name)
                || norm_dir(&o.install_dir) == norm_dir(&g.install_dir)
        });
        if !dup {
            out.push(g);
        }
    }
    out
}

/// Поля `.item`, потрібні бібліотеці. Решта ігнорується.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Item {
    #[serde(default, rename = "bIsIncompleteInstall")]
    incomplete: bool,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    install_location: String,
    #[serde(default)]
    main_window_process_name: String,
    #[serde(default)]
    app_categories: Vec<String>,
    #[serde(default)]
    app_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, categories: &[&str], main: &str, launch: &str, incomplete: bool) -> String {
        serde_json::json!({
            "FormatVersion": 0,
            "bIsIncompleteInstall": incomplete,
            "LaunchExecutable": launch,
            "DisplayName": name,
            "InstallLocation": format!(r"F:\EpicGames\{name}"),
            "MainWindowProcessName": main,
            "AppCategories": categories,
            "AppName": name.replace(' ', ""),
        })
        .to_string()
    }

    #[test]
    fn item_game_is_parsed() {
        let text = item(
            "Red Dead Redemption 2",
            &["public", "games", "applications"],
            "RDR2.exe",
            "PlayRDR2.exe",
            false,
        );
        let g = parse_item(&text).unwrap().unwrap();
        assert_eq!(g.name, "Red Dead Redemption 2");
        assert_eq!(g.app_name, "RedDeadRedemption2");
        assert_eq!(g.install_dir, r"F:\EpicGames\Red Dead Redemption 2");
        assert_eq!(g.exe_hint.as_deref(), Some("RDR2.exe"));
    }

    #[test]
    fn item_empty_main_window_has_no_hint() {
        let text = item(
            "Witch It",
            &["games", "applications"],
            "",
            "WitchIt.exe",
            false,
        );
        assert_eq!(parse_item(&text).unwrap().unwrap().exe_hint, None);
    }

    #[test]
    fn item_launch_executable_is_never_hint() {
        let text = item("Rdr", &["games"], "", "PlayRDR2.exe", false);
        assert_eq!(parse_item(&text).unwrap().unwrap().exe_hint, None);
    }

    #[test]
    fn item_engine_is_skipped() {
        let text = item(
            "Unreal Engine",
            &["engines/ue5", "engines"],
            "",
            "x.exe",
            false,
        );
        assert_eq!(parse_item(&text).unwrap(), None);
    }

    #[test]
    fn item_plugin_is_skipped() {
        let text = item(
            "Quixel Bridge",
            &["plugins/engine", "plugins"],
            "",
            "",
            false,
        );
        assert_eq!(parse_item(&text).unwrap(), None);
    }

    #[test]
    fn item_incomplete_is_skipped() {
        let text = item("Witch It", &["games"], "", "WitchIt.exe", true);
        assert_eq!(parse_item(&text).unwrap(), None);
    }

    #[test]
    fn item_without_install_location_is_skipped() {
        let text = r#"{"DisplayName":"X","AppCategories":["games"],"AppName":"X"}"#;
        assert_eq!(parse_item(text).unwrap(), None);
    }

    #[test]
    fn item_broken_json_is_error() {
        assert!(parse_item("{ не json").is_err());
    }

    #[test]
    fn epic_dedups_same_install() {
        let a = parse_item(&item("Witch It", &["games"], "", "", false))
            .unwrap()
            .unwrap();
        let mut b = a.clone();
        b.app_name = "Other".into();
        b.install_dir = b.install_dir.to_uppercase();
        let mut c = a.clone();
        c.install_dir = r"F:\Elsewhere".into();
        let d = parse_item(&item(
            "Genshin Impact",
            &["games"],
            "GenshinImpact.exe",
            "",
            false,
        ))
        .unwrap()
        .unwrap();
        let out = dedup(vec![a.clone(), b, c, d.clone()]);
        assert_eq!(out, vec![a, d]);
    }
}
