//! Профілі ігор. Профіль — це згода користувача: застосовується лише те, що в ньому ввімкнено.

use crate::GameModeError;
use optimax_core::process::{is_protected_process, PriorityClass};
use optimax_core::service::is_stoppable_service;
use optimax_core::Text;
use optimax_upscale::UpscaleSettings;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Пріоритет гри.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GamePriority {
    /// Не змінювати.
    Unchanged,
    /// Вище середнього (рекомендовано).
    #[default]
    AboveNormal,
    /// Високий — лише за явним вибором: може погіршити звук, ввід і запис/стрім.
    High,
}

impl GamePriority {
    /// Клас пріоритету Windows.
    pub fn class(self) -> Option<PriorityClass> {
        match self {
            Self::Unchanged => None,
            Self::AboveNormal => Some(PriorityClass::AboveNormal),
            Self::High => Some(PriorityClass::High),
        }
    }
}

/// Профіль гри.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GameProfile {
    /// Ім'я exe гри, напр. `GeometryDash.exe`.
    pub exe: String,
    /// Пріоритет гри.
    #[serde(default)]
    pub game_priority: GamePriority,
    /// Процеси, яким під час гри знижується пріоритет до «нижче середнього».
    #[serde(default)]
    pub lower_background: Vec<String>,
    /// Служби, які тимчасово зупиняються (лише зі списку дозволених).
    #[serde(default)]
    pub stop_services: Vec<String>,
    /// Очищати standby-пам'ять на старті гри, **якщо вільної RAM мало**.
    #[serde(default)]
    pub purge_standby_when_low: bool,
    /// Власні налаштування апскейлу для цієї гри; `None` — загальні з налаштувань програми.
    /// Систему не змінює, тож у перегляді змін режиму гри не з'являється.
    #[serde(default)]
    pub upscale: Option<UpscaleSettings>,
}

fn valid_exe_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.len() <= 260
        && n.to_ascii_lowercase().ends_with(".exe")
        && !n
            .chars()
            .any(|c| matches!(c, '\\' | '/' | ':' | '"' | '\0'))
}

impl GameProfile {
    /// Профіль із рекомендованими налаштуваннями: лише пріоритет гри «вище середнього».
    pub fn new(exe: impl Into<String>) -> Self {
        Self {
            exe: exe.into(),
            game_priority: GamePriority::default(),
            lower_background: Vec::new(),
            stop_services: Vec::new(),
            purge_standby_when_low: false,
            upscale: None,
        }
    }

    /// Безпечна копія профілю: прибирає захищені процеси, саму гру зі списку фону,
    /// недозволені служби й дублікати. Повертає попередження про кожне виправлення.
    pub fn sanitized(&self) -> Result<(Self, Vec<Text>), GameModeError> {
        if !valid_exe_name(&self.exe) {
            return Err(GameModeError::InvalidProfile(format!(
                "«{}» — не ім'я exe-файлу",
                self.exe
            )));
        }
        if is_protected_process(&self.exe) {
            return Err(GameModeError::InvalidProfile(format!(
                "{} — системний або захищений процес",
                self.exe
            )));
        }
        let mut warnings = Vec::new();
        let mut background: Vec<String> = Vec::new();
        for name in &self.lower_background {
            let n = name.trim();
            if !valid_exe_name(n) {
                warnings.push(Text::new("gm.warn.bad_name").arg("name", n));
            } else if is_protected_process(n) {
                warnings.push(Text::new("gm.warn.protected").arg("name", n));
            } else if n.eq_ignore_ascii_case(&self.exe) {
                warnings.push(Text::new("gm.warn.game_in_background").arg("name", n));
            } else if !background.iter().any(|b| b.eq_ignore_ascii_case(n)) {
                background.push(n.to_string());
            }
        }
        let mut services: Vec<String> = Vec::new();
        for s in &self.stop_services {
            let s = s.trim();
            if !is_stoppable_service(s) {
                warnings.push(Text::new("gm.warn.service_not_allowed").arg("name", s));
            } else if !services.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                services.push(s.to_string());
            }
        }
        if self.game_priority == GamePriority::High {
            warnings.push(Text::new("gm.warn.high_priority"));
        }
        Ok((
            Self {
                exe: self.exe.trim().to_string(),
                game_priority: self.game_priority,
                lower_background: background,
                stop_services: services,
                purge_standby_when_low: self.purge_standby_when_low,
                upscale: self.upscale.map(|u| UpscaleSettings {
                    sharpness: if u.sharpness.is_finite() {
                        u.sharpness.clamp(0.0, 1.0)
                    } else {
                        optimax_upscale::settings::DEFAULT_SHARPNESS
                    },
                    ..u
                }),
            },
            warnings,
        ))
    }
}

/// Сховище профілів (JSON).
#[derive(Debug, Clone)]
pub struct ProfileStore {
    path: PathBuf,
}

fn store_err(e: impl std::fmt::Display) -> GameModeError {
    GameModeError::Store(e.to_string())
}

impl ProfileStore {
    /// Сховище за шляхом.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `profiles.json` у захищеній теці даних ([`optimax_core::datadir::data_dir`]).
    pub fn default_location() -> Result<Self, optimax_core::CoreError> {
        Ok(Self::new(
            optimax_core::datadir::data_dir()?.join("profiles.json"),
        ))
    }

    /// Шлях.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Усі профілі (порожньо, якщо файлу ще немає).
    pub fn load(&self) -> Result<Vec<GameProfile>, GameModeError> {
        match fs::read_to_string(&self.path) {
            Ok(s) if s.trim().is_empty() => Ok(Vec::new()),
            Ok(s) => serde_json::from_str(&s).map_err(store_err),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(store_err(e)),
        }
    }

    fn save(&self, profiles: &[GameProfile]) -> Result<(), GameModeError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(store_err)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp).map_err(store_err)?;
            f.write_all(
                serde_json::to_string_pretty(profiles)
                    .map_err(store_err)?
                    .as_bytes(),
            )
            .map_err(store_err)?;
            f.sync_all().map_err(store_err)?;
        }
        fs::rename(&tmp, &self.path).map_err(store_err)
    }

    /// Додає або замінює профіль (за ім'ям exe). Зберігається вже безпечна версія.
    pub fn upsert(&self, profile: &GameProfile) -> Result<Vec<Text>, GameModeError> {
        let (clean, warnings) = profile.sanitized()?;
        let mut all = self.load()?;
        all.retain(|p| !p.exe.eq_ignore_ascii_case(&clean.exe));
        all.push(clean);
        self.save(&all)?;
        Ok(warnings)
    }

    /// Видаляє профіль. `false`, якщо такого не було.
    pub fn remove(&self, exe: &str) -> Result<bool, GameModeError> {
        let mut all = self.load()?;
        let before = all.len();
        all.retain(|p| !p.exe.eq_ignore_ascii_case(exe));
        let removed = all.len() != before;
        if removed {
            self.save(&all)?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_unsafe_entries() {
        let p = GameProfile {
            exe: "Game.exe".into(),
            game_priority: GamePriority::High,
            lower_background: vec![
                "chrome.exe".into(),
                "Chrome.exe".into(),
                "audiodg.exe".into(),
                "game.exe".into(),
                r"C:\x.exe".into(),
            ],
            stop_services: vec!["WSearch".into(), "wuauserv".into(), "WinDefend".into()],
            purge_standby_when_low: true,
            upscale: None,
        };
        let (clean, warnings) = p.sanitized().unwrap();
        assert_eq!(clean.lower_background, vec!["chrome.exe"]);
        assert_eq!(clean.stop_services, vec!["WSearch"]);
        let keys: Vec<&str> = warnings.iter().map(|w| w.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "gm.warn.protected",
                "gm.warn.game_in_background",
                "gm.warn.bad_name",
                "gm.warn.service_not_allowed",
                "gm.warn.service_not_allowed",
                "gm.warn.high_priority",
            ]
        );
    }

    #[test]
    fn upscale_settings_are_kept_and_clamped() {
        let mut p = GameProfile::new("Game.exe");
        p.upscale = Some(UpscaleSettings {
            filter: optimax_upscale::Filter::Nis,
            sharpness: 3.0,
            ..UpscaleSettings::default()
        });
        let (clean, warnings) = p.sanitized().unwrap();
        let up = clean.upscale.unwrap();
        assert_eq!(up.filter, optimax_upscale::Filter::Nis);
        assert_eq!(up.sharpness, 1.0);
        assert!(warnings.is_empty());
    }

    #[test]
    fn profiles_without_upscale_still_load() {
        let p: GameProfile = serde_json::from_str(r#"{"exe":"Old.exe"}"#).unwrap();
        assert_eq!(p.upscale, None);
    }

    #[test]
    fn invalid_game_names_rejected() {
        for exe in ["", "game", r"..\game.exe", "csrss.exe"] {
            assert!(GameProfile::new(exe).sanitized().is_err(), "{exe}");
        }
        assert_eq!(
            GameProfile::new("x.exe").game_priority,
            GamePriority::AboveNormal
        );
    }

    #[test]
    fn store_upsert_and_remove() {
        let dir = std::env::temp_dir().join(format!("optimax-profiles-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = ProfileStore::new(dir.join("profiles.json"));
        assert!(store.load().unwrap().is_empty());
        store.upsert(&GameProfile::new("a.exe")).unwrap();
        let mut b = GameProfile::new("b.exe");
        b.lower_background = vec!["chrome.exe".into()];
        store.upsert(&b).unwrap();
        b.purge_standby_when_low = true;
        store.upsert(&b).unwrap();
        let all = store.load().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all
            .iter()
            .any(|p| p.exe == "b.exe" && p.purge_standby_when_low));
        assert!(store.remove("A.EXE").unwrap());
        assert!(!store.remove("a.exe").unwrap());
        assert_eq!(store.load().unwrap().len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
