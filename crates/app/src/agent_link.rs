//! Обмін вікна програми з агентом у треї (етап 4б `docs/SERVICE.md`): три маленькі JSON-файли в
//! теці даних інтерфейсу користувача (`agent\`), без нового каналу.
//!
//! - `config.json` ([`AgentConfig`]) — що агенту робити: стежити за іграми, дозвіл без точки
//!   відновлення. Пишуть і вікно, і агент (меню трею).
//! - `status.json` ([`AgentStatus`]) — що агент робить. Пише лише агент; вікно читає й перетворює
//!   різницю на ті самі події `gamemode` / `blackbox`, що й без агента ([`events_between`]).
//! - `pause.json` ([`PauseMarker`]) — вікно записує гру (діагностика, A/B…): чорна скринька агента
//!   на паузі, поки живий процес із цим PID.
//!
//! Вікно віддає режим гри агенту лише без прав адміністратора ([`agent_active`]): процес з правами
//! працює з іншою текою даних і в теку користувача не пише.

use crate::dto::{GameModeEvent, RollbackResult};
use crate::session::blackbox::BlackBoxView;
use optimax_core::Text;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// М'ютекс «вікно OptimaX уже відкрите» (простір імен сеансу; займає вікно програми).
pub const UI_MUTEX: &str = r"Local\OptimaX-UI";

const DIR: &str = "agent";
const CONFIG: &str = "config.json";
const STATUS: &str = "status.json";
const PAUSE: &str = "pause.json";
/// Найбільший файл, який читаємо (стан агента — кілька КБ).
const LIMIT: u64 = 1024 * 1024;

/// Що агенту робити.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Стежити за іграми зі списку профілів (режим гри й чорна скринька).
    #[serde(default)]
    pub gamemode: bool,
    /// Дозвіл працювати без точки відновлення (агент скидає при старті).
    #[serde(default)]
    pub allow_without_restore: bool,
}

/// Гра, для якої зараз увімкнено режим гри.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveGame {
    /// Гра.
    pub game: String,
    /// PID.
    pub pid: u32,
    /// Що застосовано.
    pub applied: Vec<Text>,
    /// Що пропущено.
    pub skipped: Vec<Text>,
}

/// Гра, після якої все повернуто.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndedGame {
    /// Гра.
    pub game: String,
    /// Результат відкату.
    pub result: RollbackResult,
}

/// Режим гри чекає згоди без точки відновлення.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    /// Гра.
    pub game: String,
    /// Чому точку не створено.
    pub reason: String,
}

/// Що агент робить зараз.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentStatus {
    /// Номер зміни (росте з кожним записом).
    pub seq: u64,
    /// Служба OptimaX доступна агенту.
    pub service: bool,
    /// Стежить за іграми.
    pub watching: bool,
    /// Ігри, за якими стежить.
    pub games: Vec<String>,
    /// Гра, для якої режим гри ввімкнено зараз.
    pub active: Option<ActiveGame>,
    /// Остання гра, після якої все повернуто.
    pub last_ended: Option<EndedGame>,
    /// Чекає згоди без точки відновлення.
    pub needs_confirmation: Option<Confirmation>,
    /// Чорна скринька.
    pub blackbox: BlackBoxView,
}

impl AgentStatus {
    /// Оновлює стан за подією режиму гри.
    pub fn apply(&mut self, event: &GameModeEvent) {
        match event {
            GameModeEvent::Watching { games } => {
                self.watching = true;
                self.games.clone_from(games);
            }
            GameModeEvent::Started {
                game,
                pid,
                applied,
                skipped,
            } => {
                self.needs_confirmation = None;
                self.active = Some(ActiveGame {
                    game: game.clone(),
                    pid: *pid,
                    applied: applied.clone(),
                    skipped: skipped.clone(),
                });
            }
            GameModeEvent::NeedsConfirmation { game, reason } => {
                self.needs_confirmation = Some(Confirmation {
                    game: game.clone(),
                    reason: reason.clone(),
                });
            }
            GameModeEvent::Ended { game, result } => {
                self.active = None;
                self.last_ended = Some(EndedGame {
                    game: game.clone(),
                    result: result.clone(),
                });
            }
            GameModeEvent::Stopped => {
                self.watching = false;
                self.games.clear();
                self.active = None;
                self.needs_confirmation = None;
            }
        }
    }
}

/// Позначка паузи чорної скриньки агента.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseMarker {
    /// Процес вікна, що записує гру.
    pub pid: u32,
}

/// Події інтерфейсу з різниці двох станів агента (`old` — що вікно вже показало). Друге —
/// чи змінилась чорна скринька.
pub fn events_between(old: Option<&AgentStatus>, new: &AgentStatus) -> (Vec<GameModeEvent>, bool) {
    let blank = AgentStatus::default();
    let old = old.unwrap_or(&blank);
    let mut out = Vec::new();
    if new.watching && (!old.watching || old.games != new.games) {
        out.push(GameModeEvent::Watching {
            games: new.games.clone(),
        });
    }
    if new.last_ended != old.last_ended {
        if let Some(e) = &new.last_ended {
            out.push(GameModeEvent::Ended {
                game: e.game.clone(),
                result: e.result.clone(),
            });
        }
    }
    if new.active != old.active {
        if let Some(a) = &new.active {
            out.push(GameModeEvent::Started {
                game: a.game.clone(),
                pid: a.pid,
                applied: a.applied.clone(),
                skipped: a.skipped.clone(),
            });
        }
    }
    if new.needs_confirmation != old.needs_confirmation {
        if let Some(c) = &new.needs_confirmation {
            out.push(GameModeEvent::NeedsConfirmation {
                game: c.game.clone(),
                reason: c.reason.clone(),
            });
        }
    }
    if old.watching && !new.watching {
        out.push(GameModeEvent::Stopped);
    }
    (out, new.blackbox != old.blackbox)
}

fn path(name: &str) -> Option<PathBuf> {
    optimax_core::datadir::ui_dir()
        .ok()
        .map(|d| d.join(DIR).join(name))
}

fn read<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    let p = path(name)?;
    let bytes = optimax_core::safefs::read_limited(&p, LIMIT).ok()??;
    serde_json::from_slice(&bytes).ok()
}

/// Запис цілим файлом через тимчасовий (щоб інша сторона не прочитала половину).
fn write<T: Serialize>(name: &str, value: &T) -> std::io::Result<()> {
    let p = path(name).ok_or_else(|| std::io::Error::other("тека даних недоступна"))?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("tmp");
    let text = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &p)
}

/// Налаштування агента (немає файлу — усе вимкнено).
pub fn read_config() -> AgentConfig {
    read(CONFIG).unwrap_or_default()
}

/// Записує налаштування агента.
pub fn write_config(cfg: &AgentConfig) -> std::io::Result<()> {
    write(CONFIG, cfg)
}

/// Змінює налаштування агента.
pub fn update_config(f: impl FnOnce(&mut AgentConfig)) -> std::io::Result<()> {
    let mut cfg = read_config();
    f(&mut cfg);
    write_config(&cfg)
}

/// Стан агента (останній записаний).
pub fn read_status() -> Option<AgentStatus> {
    read(STATUS)
}

/// Записує стан агента.
pub fn write_status(status: &AgentStatus) -> std::io::Result<()> {
    write(STATUS, status)
}

/// Ставить або знімає паузу чорної скриньки агента від цього процесу.
pub fn set_pause(on: bool) {
    let r = if on {
        write(
            PAUSE,
            &PauseMarker {
                pid: std::process::id(),
            },
        )
    } else {
        match path(PAUSE) {
            Some(p) => match std::fs::remove_file(p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    };
    if let Err(e) = r {
        tracing::warn!("пауза чорної скриньки агента: {e}");
    }
}

/// Чи вікно програми просить паузу (позначка є, і її процес живий).
pub fn pause_active() -> bool {
    read::<PauseMarker>(PAUSE)
        .is_some_and(|m| !optimax_core::process::wait_exit(m.pid, std::time::Duration::ZERO))
}

/// Режим гри веде агент: він працює в цьому сеансі, а це вікно — без прав адміністратора (з
/// правами тека даних інша, і обмінятися не вийде).
pub fn agent_active() -> bool {
    !optimax_core::process::is_elevated() && crate::agent::is_running()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ended(game: &str) -> EndedGame {
        EndedGame {
            game: game.into(),
            result: RollbackResult::default(),
        }
    }

    fn active(game: &str, pid: u32) -> ActiveGame {
        ActiveGame {
            game: game.into(),
            pid,
            applied: vec![Text::new("gm.applied.gpu")],
            skipped: Vec::new(),
        }
    }

    #[test]
    fn status_follows_events_and_diff_gives_them_back() {
        let mut s = AgentStatus::default();
        let events = [
            GameModeEvent::Watching {
                games: vec!["re9.exe".into()],
            },
            GameModeEvent::Started {
                game: "re9.exe".into(),
                pid: 7,
                applied: vec![Text::new("gm.applied.gpu")],
                skipped: Vec::new(),
            },
        ];
        let mut prev = s.clone();
        for e in &events {
            s.apply(e);
            let (out, _) = events_between(Some(&prev), &s);
            assert_eq!(out, vec![e.clone()]);
            prev = s.clone();
        }
        s.apply(&GameModeEvent::Ended {
            game: "re9.exe".into(),
            result: RollbackResult::default(),
        });
        let (out, _) = events_between(Some(&prev), &s);
        assert_eq!(
            out,
            vec![GameModeEvent::Ended {
                game: "re9.exe".into(),
                result: RollbackResult::default()
            }]
        );
        let prev = s.clone();
        s.apply(&GameModeEvent::Stopped);
        assert_eq!(
            events_between(Some(&prev), &s).0,
            vec![GameModeEvent::Stopped]
        );
    }

    #[test]
    fn window_opened_mid_game_gets_watching_and_started() {
        let s = AgentStatus {
            watching: true,
            games: vec!["re9.exe".into()],
            active: Some(active("re9.exe", 7)),
            ..Default::default()
        };
        let (out, _) = events_between(None, &s);
        assert!(matches!(out[0], GameModeEvent::Watching { .. }));
        assert!(matches!(out[1], GameModeEvent::Started { pid: 7, .. }));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn next_game_right_after_previous_gives_ended_then_started() {
        let old = AgentStatus {
            watching: true,
            games: vec!["a.exe".into(), "b.exe".into()],
            active: Some(active("a.exe", 1)),
            ..Default::default()
        };
        let new = AgentStatus {
            active: Some(active("b.exe", 2)),
            last_ended: Some(ended("a.exe")),
            ..old.clone()
        };
        let (out, bb) = events_between(Some(&old), &new);
        assert!(matches!(&out[0], GameModeEvent::Ended { game, .. } if game == "a.exe"));
        assert!(matches!(&out[1], GameModeEvent::Started { game, .. } if game == "b.exe"));
        assert!(!bb);
    }

    #[test]
    fn nothing_changed_gives_nothing() {
        let s = AgentStatus {
            watching: true,
            games: vec!["a.exe".into()],
            ..Default::default()
        };
        assert_eq!(events_between(Some(&s), &s), (Vec::new(), false));
    }

    #[test]
    fn config_survives_unknown_and_missing_fields() {
        let cfg: AgentConfig = serde_json::from_str(r#"{"gamemode":true,"future":1}"#).unwrap();
        assert!(cfg.gamemode && !cfg.allow_without_restore);
        let cfg: AgentConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg, AgentConfig::default());
    }
}
