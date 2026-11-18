//! Скан бібліотеки: Steam, Epic, GOG, Battle.net → один список без дублікатів.
//!
//! Кожна помилка джерела чи файлу потрапляє в [`LibraryReport::problems`], а решта списку
//! будується. Гра показується лише тоді, коли її тека встановлення існує.

use super::fs::LibraryFs;
use super::store::LibraryStore;
use super::{epic, norm_dir, registry, steam, GameEntry, Launcher, LibraryProblem, LibraryReport};
use optimax_core::registry::{RegHive, RegValue, RegistryApi};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Звідки читати файли лаунчерів.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources {
    /// Корінь Steam (`None` — Steam не встановлено).
    pub steam_root: Option<PathBuf>,
    /// Тека маніфестів Epic (`None` — невідома).
    pub epic_manifests: Option<PathBuf>,
}

/// Корінь Steam: `HKCU\Software\Valve\Steam\SteamPath`, запасний —
/// `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\InstallPath`.
pub fn steam_root(reg: &dyn RegistryApi) -> Option<PathBuf> {
    let read = |hive, key, name| match reg.read_value(hive, key, name) {
        Ok(Some(RegValue::String(v) | RegValue::ExpandString(v))) if !v.trim().is_empty() => {
            Some(PathBuf::from(v.trim().replace('/', "\\")))
        }
        _ => None,
    };
    read(RegHive::CurrentUser, r"Software\Valve\Steam", "SteamPath").or_else(|| {
        read(
            RegHive::LocalMachine,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        )
    })
}

/// `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`.
pub fn default_epic_manifests() -> Option<PathBuf> {
    let base = std::env::var_os("ProgramData")?;
    Some(
        PathBuf::from(base)
            .join("Epic")
            .join("EpicGamesLauncher")
            .join("Data")
            .join("Manifests"),
    )
}

/// Будує список ігор.
pub fn scan_library(
    fs: &dyn LibraryFs,
    reg: &dyn RegistryApi,
    src: &Sources,
    store: &LibraryStore,
) -> LibraryReport {
    let mut games = Vec::new();
    let mut problems = Vec::new();
    if let Some(root) = &src.steam_root {
        scan_steam(fs, root, &mut games, &mut problems);
    }
    if let Some(dir) = &src.epic_manifests {
        scan_epic(fs, dir, &mut games, &mut problems);
    }
    let from_registry: [(Launcher, &str, RegScan); 2] = [
        (Launcher::Gog, "GOG.com\\Games", registry::gog_games),
        (Launcher::Battlenet, "Uninstall", registry::battlenet_games),
    ];
    for (launcher, what, read) in from_registry {
        match read(reg) {
            Ok(found) => games.extend(
                found
                    .into_iter()
                    .filter(|g| fs.is_dir(Path::new(&g.install_dir)))
                    .map(|g| entry(g.key, g.name, launcher, g.install_dir, g.exe_hint, false)),
            ),
            Err(e) => problems.push(problem(launcher, what, e.to_string())),
        }
    }

    let mut seen = HashSet::new();
    games.retain(|g: &GameEntry| seen.insert(norm_dir(&g.install_dir)));
    for g in &mut games {
        if let Some(u) = store.games.get(&g.id) {
            g.exe.clone_from(&u.exe);
            g.hidden = u.hidden;
        }
    }
    for m in &store.manual {
        let mut g = entry(
            m.id.clone(),
            m.name.clone(),
            Launcher::Manual,
            String::new(),
            None,
            false,
        );
        g.exe = Some(m.exe.clone());
        g.hidden = store.games.get(&m.id).is_some_and(|u| u.hidden);
        games.push(g);
    }
    games.sort_by_cached_key(|g| g.name.to_lowercase());
    LibraryReport { games, problems }
}

/// Шлях до обкладинки гри Steam (`steam:<appid>`, appid — лише цифри); для решти — `None`.
pub fn cover_path(src: &Sources, id: &str) -> Option<PathBuf> {
    let appid = id.strip_prefix("steam:")?;
    if appid.is_empty() || !appid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(
        src.steam_root
            .as_ref()?
            .join("appcache")
            .join("librarycache")
            .join(appid)
            .join(COVER_FILE),
    )
}

/// Ім'я вертикальної обкладинки в кеші Steam.
const COVER_FILE: &str = "library_600x900.jpg";

/// Наявна обкладинка гри Steam: `<appid>\library_600x900.jpg` або, у новіших клієнтах Steam,
/// `<appid>\<хеш>\library_600x900.jpg` (обидва варіанти звірено 2026-11-18).
pub fn find_cover(fs: &dyn LibraryFs, src: &Sources, id: &str) -> Option<PathBuf> {
    let direct = cover_path(src, id)?;
    if fs.is_file(&direct) {
        return Some(direct);
    }
    let dir = direct.parent()?;
    fs.list_dirs(dir)
        .ok()?
        .into_iter()
        .map(|d| d.join(COVER_FILE))
        .find(|p| fs.is_file(p))
}

type RegScan = fn(&dyn RegistryApi) -> Result<Vec<registry::RegGame>, optimax_core::CoreError>;

fn entry(
    id: String,
    name: String,
    launcher: Launcher,
    install_dir: String,
    exe_hint: Option<String>,
    has_cover: bool,
) -> GameEntry {
    GameEntry {
        id,
        name,
        launcher,
        install_dir,
        exe: None,
        exe_hint,
        has_cover,
        hidden: false,
    }
}

fn problem(
    launcher: Launcher,
    what: impl Into<String>,
    reason: impl Into<String>,
) -> LibraryProblem {
    LibraryProblem {
        launcher,
        what: what.into(),
        reason: reason.into(),
    }
}

fn scan_steam(
    fs: &dyn LibraryFs,
    root: &Path,
    games: &mut Vec<GameEntry>,
    problems: &mut Vec<LibraryProblem>,
) {
    let folders = root.join("steamapps").join("libraryfolders.vdf");
    // Немає файлу — Steam видалено, а запис у реєстрі лишився; це не проблема.
    if !fs.is_file(&folders) {
        return;
    }
    let libs = match fs
        .read_to_string(&folders)
        .map_err(|e| e.to_string())
        .and_then(|t| steam::library_paths(&t).map_err(|e| e.to_string()))
    {
        Ok(libs) => libs,
        Err(e) => {
            problems.push(problem(Launcher::Steam, folders.display().to_string(), e));
            return;
        }
    };
    let src = Sources {
        steam_root: Some(root.to_path_buf()),
        epic_manifests: None,
    };
    for lib in libs {
        let apps = Path::new(&lib).join("steamapps");
        let files = match fs.list_files(&apps, "acf") {
            Ok(f) => f,
            Err(e) => {
                problems.push(problem(
                    Launcher::Steam,
                    lib.clone(),
                    format!("бібліотека недоступна (диск відключено?): {e}"),
                ));
                continue;
            }
        };
        for file in files {
            let is_manifest = file
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_ascii_lowercase().starts_with("appmanifest_"));
            if !is_manifest {
                continue;
            }
            let m = match fs
                .read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|t| steam::parse_appmanifest(&t).map_err(|e| e.to_string()))
            {
                Ok(m) => m,
                Err(e) => {
                    problems.push(problem(Launcher::Steam, file.display().to_string(), e));
                    continue;
                }
            };
            if steam::EXCLUDED_APPIDS.contains(&m.appid.as_str()) {
                continue;
            }
            let dir = apps.join("common").join(&m.installdir);
            if !fs.is_dir(&dir) {
                continue;
            }
            let id = format!("steam:{}", m.appid);
            let has_cover = find_cover(fs, &src, &id).is_some();
            let install_dir = dir.to_string_lossy().into_owned();
            games.push(entry(
                id,
                m.name,
                Launcher::Steam,
                install_dir,
                None,
                has_cover,
            ));
        }
    }
}

fn scan_epic(
    fs: &dyn LibraryFs,
    dir: &Path,
    games: &mut Vec<GameEntry>,
    problems: &mut Vec<LibraryProblem>,
) {
    // Немає теки — Epic не встановлено; це не проблема.
    if !fs.is_dir(dir) {
        return;
    }
    let files = match fs.list_files(dir, "item") {
        Ok(f) => f,
        Err(e) => {
            problems.push(problem(
                Launcher::Epic,
                dir.display().to_string(),
                e.to_string(),
            ));
            return;
        }
    };
    let mut found = Vec::new();
    for file in files {
        match fs
            .read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|t| epic::parse_item(&t).map_err(|e| e.to_string()))
        {
            Ok(Some(g)) => found.push(g),
            Ok(None) => {}
            Err(e) => problems.push(problem(Launcher::Epic, file.display().to_string(), e)),
        }
    }
    // Спершу — лише наявні теки: інакше застарілий маніфест (стара тека після перенесення)
    // міг би перемогти в дедуплікації, а потім зникнути, і гра пропала б зовсім.
    found.retain(|g| fs.is_dir(Path::new(&g.install_dir)));
    games.extend(epic::dedup(found).into_iter().map(|g| {
        let key = if g.app_name.is_empty() {
            &g.name
        } else {
            &g.app_name
        };
        entry(
            format!("epic:{key}"),
            g.name.clone(),
            Launcher::Epic,
            g.install_dir,
            g.exe_hint,
            false,
        )
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::fs::MemFs;
    use crate::library::registry::{GOG_ROOT, UNINSTALL_ROOTS};
    use crate::library::store::UserGame;
    use optimax_core::registry::{MemRegistry, RegHive, RegValue};

    const FOLDERS: &str = r#""libraryfolders"
{
	"0" { "path" "D:\\Steam" }
	"1" { "path" "E:\\SteamLibrary" }
}"#;

    fn manifest(appid: &str, name: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"name\"\t\t\"{name}\"\n\t\"installdir\"\t\t\"{name}\"\n}}\n"
        )
    }

    fn src() -> Sources {
        Sources {
            steam_root: Some(PathBuf::from(r"D:\Steam")),
            epic_manifests: Some(PathBuf::from(
                r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests",
            )),
        }
    }

    /// Steam на D: (дві гри, одна з обкладинкою) і E: (одна гра).
    fn steam_fs() -> MemFs {
        let mut fs = MemFs::new();
        fs.file(r"D:\Steam\steamapps\libraryfolders.vdf", FOLDERS)
            .file(
                r"D:\Steam\steamapps\appmanifest_1293230.acf",
                manifest("1293230", "House of Detention"),
            )
            .dir(r"D:\Steam\steamapps\common\House of Detention")
            .file(
                r"D:\Steam\appcache\librarycache\1293230\library_600x900.jpg",
                vec![1u8, 2, 3],
            )
            .file(
                r"D:\Steam\steamapps\appmanifest_228980.acf",
                manifest("228980", "Steamworks Common Redistributables"),
            )
            .dir(r"D:\Steam\steamapps\common\Steamworks Common Redistributables")
            .file(
                r"E:\SteamLibrary\steamapps\appmanifest_292030.acf",
                manifest("292030", "The Witcher 3"),
            )
            .dir(r"E:\SteamLibrary\steamapps\common\The Witcher 3");
        fs
    }

    fn names(r: &LibraryReport) -> Vec<&str> {
        r.games.iter().map(|g| g.name.as_str()).collect()
    }

    fn scan(fs: &MemFs, reg: &MemRegistry, store: &LibraryStore) -> LibraryReport {
        scan_library(fs, reg, &src(), store)
    }

    #[test]
    fn scan_steam_two_libraries() {
        let r = scan(&steam_fs(), &MemRegistry::new(), &LibraryStore::default());
        assert_eq!(names(&r), vec!["House of Detention", "The Witcher 3"]);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        let hod = &r.games[0];
        assert_eq!(hod.id, "steam:1293230");
        assert_eq!(hod.launcher, Launcher::Steam);
        assert!(hod.has_cover);
        assert!(!r.games[1].has_cover);
        assert!(
            r.games[1].install_dir.ends_with(r"common\The Witcher 3"),
            "{}",
            r.games[1].install_dir
        );
    }

    #[test]
    fn scan_skips_missing_common_dir() {
        let mut fs = steam_fs();
        fs.file(
            r"D:\Steam\steamapps\appmanifest_5.acf",
            manifest("5", "Removed Game"),
        );
        let r = scan(&fs, &MemRegistry::new(), &LibraryStore::default());
        assert!(!names(&r).contains(&"Removed Game"));
    }

    #[test]
    fn scan_skips_excluded_appid() {
        let r = scan(&steam_fs(), &MemRegistry::new(), &LibraryStore::default());
        assert!(!names(&r).contains(&"Steamworks Common Redistributables"));
    }

    #[test]
    fn scan_missing_library_drive_is_problem() {
        let mut fs = MemFs::new();
        fs.file(r"D:\Steam\steamapps\libraryfolders.vdf", FOLDERS)
            .file(
                r"D:\Steam\steamapps\appmanifest_1293230.acf",
                manifest("1293230", "House of Detention"),
            )
            .dir(r"D:\Steam\steamapps\common\House of Detention");
        let r = scan(&fs, &MemRegistry::new(), &LibraryStore::default());
        assert_eq!(names(&r), vec!["House of Detention"]);
        assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
        assert_eq!(r.problems[0].launcher, Launcher::Steam);
        assert!(
            r.problems[0].what.contains("SteamLibrary"),
            "{:?}",
            r.problems[0]
        );
    }

    #[test]
    fn scan_broken_appmanifest_is_problem() {
        let mut fs = steam_fs();
        fs.file(
            r"D:\Steam\steamapps\appmanifest_7.acf",
            "\"AppState\" { \"appid\" ",
        );
        let r = scan(&fs, &MemRegistry::new(), &LibraryStore::default());
        assert_eq!(r.games.len(), 2);
        assert_eq!(r.problems.len(), 1);
        assert!(
            r.problems[0].what.contains("appmanifest_7"),
            "{:?}",
            r.problems[0]
        );
    }

    #[test]
    fn scan_broken_libraryfolders_is_problem() {
        let mut fs = MemFs::new();
        fs.file(r"D:\Steam\steamapps\libraryfolders.vdf", "{{{");
        let r = scan(&fs, &MemRegistry::new(), &LibraryStore::default());
        assert!(r.games.is_empty());
        assert_eq!(r.problems.len(), 1);
    }

    fn other_launchers() -> (MemFs, MemRegistry) {
        let mut fs = MemFs::new();
        let epic = serde_json::json!({
            "DisplayName": "Red Dead Redemption 2", "AppName": "Heather",
            "InstallLocation": r"F:\EpicGames\RDR2", "MainWindowProcessName": "RDR2.exe",
            "LaunchExecutable": "PlayRDR2.exe", "AppCategories": ["games"], "bIsIncompleteInstall": false
        });
        let engine = serde_json::json!({
            "DisplayName": "Unreal Engine", "AppName": "UE_5.6",
            "InstallLocation": r"F:\EpicGames\UE_5.6", "AppCategories": ["engines"]
        });
        fs.file(
            r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests\A.item",
            epic.to_string(),
        )
        .file(
            r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests\B.item",
            engine.to_string(),
        )
        .dir(r"F:\EpicGames\RDR2")
        .dir(r"F:\EpicGames\UE_5.6")
        .dir(r"F:\Games\Kerbal Space Program")
        .dir(r"F:\Call of Duty");
        let mut reg = MemRegistry::new();
        let lm = RegHive::LocalMachine;
        let s = |v: &str| RegValue::String(v.into());
        let gog = format!(r"{GOG_ROOT}\1429864849");
        reg.set(lm, &gog, "gameName", s("Kerbal Space Program"));
        reg.set(lm, &gog, "path", s(r"F:\Games\Kerbal Space Program"));
        reg.set(lm, &gog, "exeFile", s("KSP_x64.exe"));
        let cod = format!(r"{}\Call of Duty", UNINSTALL_ROOTS[0]);
        reg.set(lm, &cod, "DisplayName", s("Call of Duty"));
        reg.set(lm, &cod, "Publisher", s("Activision"));
        reg.set(lm, &cod, "InstallLocation", s(r"F:\Call of Duty"));
        reg.set(
            lm,
            &cod,
            "DisplayIcon",
            s(r"F:\Call of Duty\_retail_\cod.exe"),
        );
        (fs, reg)
    }

    #[test]
    fn scan_epic_and_gog_and_bnet() {
        let (fs, reg) = other_launchers();
        let r = scan(&fs, &reg, &LibraryStore::default());
        assert_eq!(
            names(&r),
            vec![
                "Call of Duty",
                "Kerbal Space Program",
                "Red Dead Redemption 2"
            ]
        );
        let rdr = r
            .games
            .iter()
            .find(|g| g.launcher == Launcher::Epic)
            .unwrap();
        assert_eq!(rdr.id, "epic:Heather");
        assert_eq!(rdr.exe_hint.as_deref(), Some("RDR2.exe"));
        assert_eq!(rdr.exe, None);
        assert!(r.games.iter().any(|g| g.launcher == Launcher::Gog));
        assert!(r.games.iter().any(|g| g.launcher == Launcher::Battlenet));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
    }

    #[test]
    fn scan_skips_registry_game_without_folder() {
        let (mut fs, reg) = other_launchers();
        fs = {
            let mut f = MemFs::new();
            f.dir(r"F:\EpicGames\RDR2");
            let _ = fs;
            f
        };
        let r = scan(&fs, &reg, &LibraryStore::default());
        assert!(!names(&r).contains(&"Call of Duty"));
        assert!(!names(&r).contains(&"Kerbal Space Program"));
    }

    #[test]
    fn scan_epic_stale_manifest_does_not_hide_moved_game() {
        let item = |dir: &str| {
            serde_json::json!({
                "DisplayName": "Witch It", "AppName": "WitchIt",
                "InstallLocation": dir, "AppCategories": ["games"]
            })
            .to_string()
        };
        let mut fs = MemFs::new();
        fs.file(
            r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests\A.item",
            item(r"C:\Old\WitchIt"),
        )
        .file(
            r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests\B.item",
            item(r"F:\EpicGames\WitchIt"),
        )
        .dir(r"F:\EpicGames\WitchIt");
        let r = scan(&fs, &MemRegistry::new(), &LibraryStore::default());
        assert_eq!(names(&r), vec!["Witch It"]);
        assert_eq!(r.games[0].install_dir, r"F:\EpicGames\WitchIt");
    }

    #[test]
    fn scan_dedups_by_install_dir() {
        let mut fs = steam_fs();
        let mut reg = MemRegistry::new();
        let gog = format!(r"{GOG_ROOT}\9");
        reg.set(
            RegHive::LocalMachine,
            &gog,
            "gameName",
            RegValue::String("Witcher GOG".into()),
        );
        reg.set(
            RegHive::LocalMachine,
            &gog,
            "path",
            RegValue::String(r"e:\steamlibrary\steamapps\common\the witcher 3\".into()),
        );
        fs.dir(r"E:\SteamLibrary\steamapps\common\The Witcher 3");
        let r = scan(&fs, &reg, &LibraryStore::default());
        assert_eq!(names(&r), vec!["House of Detention", "The Witcher 3"]);
    }

    #[test]
    fn scan_applies_store() {
        let mut store = LibraryStore::default();
        store.games.insert(
            "steam:292030".into(),
            UserGame {
                exe: Some("witcher3.exe".into()),
                hidden: true,
            },
        );
        store.add_manual("Моя гра", "MyGame.exe");
        let r = scan(&steam_fs(), &MemRegistry::new(), &store);
        let w = r.games.iter().find(|g| g.id == "steam:292030").unwrap();
        assert_eq!(w.exe.as_deref(), Some("witcher3.exe"));
        assert!(w.hidden);
        let m = r
            .games
            .iter()
            .find(|g| g.launcher == Launcher::Manual)
            .unwrap();
        assert_eq!(m.name, "Моя гра");
        assert_eq!(m.exe.as_deref(), Some("MyGame.exe"));
        assert_eq!(m.install_dir, "");
    }

    #[test]
    fn store_survives_rescan() {
        let mut store = LibraryStore::default();
        store.set_exe("steam:999", Some("gone.exe".into()));
        let before = store.clone();
        let r = scan(&steam_fs(), &MemRegistry::new(), &store);
        assert!(!r.games.iter().any(|g| g.id == "steam:999"));
        assert_eq!(store, before);
    }

    #[test]
    fn scan_no_launchers_is_empty_without_problems() {
        let r = scan_library(
            &MemFs::new(),
            &MemRegistry::new(),
            &Sources::default(),
            &LibraryStore::default(),
        );
        assert!(r.games.is_empty());
        assert!(r.problems.is_empty());
    }

    #[test]
    fn scan_epic_missing_dir_is_silent() {
        let r = scan(&MemFs::new(), &MemRegistry::new(), &LibraryStore::default());
        assert!(r.games.is_empty());
        assert!(r.problems.is_empty(), "{:?}", r.problems);
    }

    #[test]
    fn steam_root_prefers_hkcu_and_normalizes_slashes() {
        let mut reg = MemRegistry::new();
        reg.set(
            RegHive::CurrentUser,
            r"Software\Valve\Steam",
            "SteamPath",
            RegValue::String("d:/steam".into()),
        );
        reg.set(
            RegHive::LocalMachine,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
            RegValue::String(r"C:\Other".into()),
        );
        assert_eq!(steam_root(&reg), Some(PathBuf::from(r"d:\steam")));
    }

    #[test]
    fn steam_root_falls_back_to_hklm() {
        let mut reg = MemRegistry::new();
        reg.set(
            RegHive::LocalMachine,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
            RegValue::String(r"D:\Steam".into()),
        );
        assert_eq!(steam_root(&reg), Some(PathBuf::from(r"D:\Steam")));
        assert_eq!(steam_root(&MemRegistry::new()), None);
    }

    #[test]
    fn cover_path_for_steam_only() {
        let p = cover_path(&src(), "steam:1293230").unwrap();
        assert!(super::super::norm_dir(&p.to_string_lossy())
            .ends_with(r"appcache\librarycache\1293230\library_600x900.jpg"));
        assert_eq!(cover_path(&src(), "epic:Heather"), None);
    }

    #[test]
    fn find_cover_in_hashed_subfolder() {
        let mut fs = MemFs::new();
        fs.file(
            r"D:\Steam\appcache\librarycache\292030\3be8f52e\library_header.jpg",
            vec![1u8],
        )
        .file(
            r"D:\Steam\appcache\librarycache\292030\8b21381a\library_600x900.jpg",
            vec![1u8],
        );
        let p = find_cover(&fs, &src(), "steam:292030").unwrap();
        assert!(norm_dir(&p.to_string_lossy()).ends_with(r"8b21381a\library_600x900.jpg"));
        assert_eq!(find_cover(&fs, &src(), "steam:1"), None);
        assert_eq!(find_cover(&fs, &src(), "gog:1"), None);
    }

    #[test]
    fn find_cover_prefers_direct_file() {
        let mut fs = MemFs::new();
        fs.file(
            r"D:\Steam\appcache\librarycache\5\library_600x900.jpg",
            vec![1u8],
        )
        .file(
            r"D:\Steam\appcache\librarycache\5\aa\library_600x900.jpg",
            vec![1u8],
        );
        let p = find_cover(&fs, &src(), "steam:5").unwrap();
        assert!(norm_dir(&p.to_string_lossy()).ends_with(r"librarycache\5\library_600x900.jpg"));
    }

    #[test]
    fn scan_has_cover_from_hashed_subfolder() {
        let mut fs = steam_fs();
        fs.file(
            r"D:\Steam\appcache\librarycache\292030\8b21381a\library_600x900.jpg",
            vec![1u8],
        );
        let r = scan(&fs, &MemRegistry::new(), &LibraryStore::default());
        assert!(
            r.games
                .iter()
                .find(|g| g.id == "steam:292030")
                .unwrap()
                .has_cover
        );
    }

    #[test]
    fn cover_path_rejects_non_numeric_appid() {
        assert_eq!(cover_path(&src(), r"steam:..\x"), None);
        assert_eq!(cover_path(&src(), "steam:"), None);
    }
}

/// Скан справжньої системи (лише читання): `cargo test -p optimax-gamemode real_scan -- --ignored --nocapture`.
#[cfg(all(test, windows))]
mod real {
    #[test]
    #[ignore = "читає справжні лаунчери цього ПК"]
    fn real_scan_prints_library() {
        let reg = optimax_core::registry::WinRegistry;
        let src = super::Sources {
            steam_root: super::steam_root(&reg),
            epic_manifests: super::default_epic_manifests(),
        };
        let r = super::scan_library(&crate::library::fs::StdFs, &reg, &src, &Default::default());
        for g in &r.games {
            println!(
                "{:?} | {} | {} | hint={:?} cover={}",
                g.launcher, g.name, g.install_dir, g.exe_hint, g.has_cover
            );
        }
        println!("problems: {:?}", r.problems);
    }
}
