//! Steam: бібліотеки з `steamapps\libraryfolders.vdf` і ігри з `appmanifest_<appid>.acf`
//! (формат звірено 2026-11-18 на реальному ПК: Steam на `D:\Steam` + `E:\SteamLibrary`).

use super::vdf::{self, Node};
use super::LibraryError;

/// appid, які не є іграми й не показуються. Лише звірені на реальній системі:
/// `228980` — «Steamworks Common Redistributables» (назва з його `appmanifest`, 2026-11-18).
pub const EXCLUDED_APPIDS: &[&str] = &["228980"];

/// Дані гри з `appmanifest_<appid>.acf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifest {
    /// appid.
    pub appid: String,
    /// Назва гри.
    pub name: String,
    /// Ім'я теки в `steamapps\common`.
    pub installdir: String,
}

/// Шляхи всіх бібліотек Steam з тексту `libraryfolders.vdf`.
pub fn library_paths(libraryfolders: &str) -> Result<Vec<String>, LibraryError> {
    const WHAT: &str = "libraryfolders.vdf";
    let root = vdf::parse(libraryfolders).map_err(|e| LibraryError::parse(WHAT, e.to_string()))?;
    let folders = root
        .get("libraryfolders")
        .ok_or_else(|| LibraryError::parse(WHAT, "немає блоку libraryfolders"))?;
    Ok(folders
        .pairs()
        .iter()
        .filter_map(|(_, lib)| lib.get("path").and_then(Node::str))
        .filter(|p| !p.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// Розбирає `appmanifest_<appid>.acf`. Без `appid`, `name` чи `installdir` — помилка.
pub fn parse_appmanifest(text: &str) -> Result<AppManifest, LibraryError> {
    const WHAT: &str = "appmanifest";
    let root = vdf::parse(text).map_err(|e| LibraryError::parse(WHAT, e.to_string()))?;
    let state = root
        .get("AppState")
        .ok_or_else(|| LibraryError::parse(WHAT, "немає блоку AppState"))?;
    let field = |key: &'static str| {
        state
            .get(key)
            .and_then(Node::str)
            .filter(|v| !v.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| LibraryError::parse(WHAT, format!("немає поля {key}")))
    };
    Ok(AppManifest {
        appid: field("appid")?,
        name: field("name")?,
        installdir: field("installdir")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDERS: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"D:\\Steam"
		"apps"
		{
			"1293230"		"8912422542"
		}
	}
	"1"
	{
		"path"		"E:\\SteamLibrary"
	}
}
"#;

    const MANIFEST: &str = r#""AppState"
{
	"appid"		"1293230"
	"Universe"		"1"
	"LauncherPath"		"D:\\Steam\\steam.exe"
	"name"		"House of Detention"
	"StateFlags"		"4"
	"installdir"		"House of Detention"
	"SizeOnDisk"		"8912422542"
}
"#;

    #[test]
    fn library_paths_reads_all_folders() {
        assert_eq!(
            library_paths(FOLDERS).unwrap(),
            vec![r"D:\Steam".to_string(), r"E:\SteamLibrary".to_string()]
        );
    }

    #[test]
    fn library_paths_without_root_is_error() {
        assert!(library_paths("\"other\" { }").is_err());
    }

    #[test]
    fn appmanifest_reads_fields() {
        let m = parse_appmanifest(MANIFEST).unwrap();
        assert_eq!(
            m,
            AppManifest {
                appid: "1293230".into(),
                name: "House of Detention".into(),
                installdir: "House of Detention".into(),
            }
        );
    }

    #[test]
    fn appmanifest_missing_name_is_error() {
        let text = MANIFEST.replace("\"name\"\t\t\"House of Detention\"\n", "");
        assert!(parse_appmanifest(&text).is_err());
    }
}
