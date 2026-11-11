//! Профілі ігор у службі (етап 3г з `docs/SERVICE.md`).
//!
//! Профіль визначає, що режим гри змінює під час гри (пріоритет, служби, standby), тож лежить у
//! захищеній теці користувача (`%ProgramData%\OptimaX\<SID>\profiles.json`), куди програма без
//! прав адміністратора писати не може. Служба зберігає лише безпечну версію профілю
//! ([`GameProfile::sanitized`]); самі зміни все одно перевіряє рушій твіків.

use optimax_gamemode::{GameModeError, GameProfile, ProfileStore};
use optimax_service::protocol::{Command, Reply};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

/// Ім'я файлу профілів у теці даних користувача.
pub const PROFILES_FILE: &str = "profiles.json";

/// Тека даних користувача за SID.
pub type DirFor = Box<dyn Fn(&str) -> Result<PathBuf, String> + Send + Sync>;

/// Профілі всіх користувачів, що під'єднувались.
pub struct ProfilesHost {
    dir_for: DirFor,
    /// Читання-зміна-запис файлу — по одному.
    lock: Mutex<()>,
}

fn json<T: Serialize>(v: &T) -> Reply {
    serde_json::to_value(v).map_or_else(|e| Reply::Error(e.to_string()), Reply::Json)
}

fn reply<T: Serialize>(r: Result<T, GameModeError>) -> Reply {
    r.map_or_else(|e| Reply::Error(e.to_string()), |v| json(&v))
}

impl ProfilesHost {
    /// Теки даних — з `dir_for` (у службі: `datadir::data_dir_for`).
    pub fn new(dir_for: DirFor) -> Self {
        Self {
            dir_for,
            lock: Mutex::new(()),
        }
    }

    /// Виконує команду профілів від користувача `sid`; `None` — команда не про профілі.
    pub fn handle(&self, sid: &str, cmd: &Command) -> Option<Reply> {
        if !matches!(
            cmd,
            Command::ProfilesList | Command::ProfileSave { .. } | Command::ProfileRemove { .. }
        ) {
            return None;
        }
        let store = match (self.dir_for)(sid) {
            Ok(dir) => ProfileStore::new(dir.join(PROFILES_FILE)),
            Err(e) => return Some(Reply::Error(format!("профілі недоступні: {e}"))),
        };
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        Some(match cmd {
            Command::ProfilesList => reply(store.load()),
            Command::ProfileSave { profile } => {
                match serde_json::from_value::<GameProfile>(profile.clone()) {
                    Ok(p) => {
                        tracing::info!("служба: профіль {} для {sid}", p.exe);
                        reply(store.upsert(&p))
                    }
                    Err(e) => Reply::Error(format!("некоректний профіль: {e}")),
                }
            }
            Command::ProfileRemove { exe } => reply(store.remove(exe)),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_gamemode::GamePriority;

    fn host(base: &std::path::Path) -> ProfilesHost {
        let base = base.to_path_buf();
        ProfilesHost::new(Box::new(move |sid: &str| {
            if optimax_core::datadir::is_plain_sid(sid) {
                Ok(base.join(sid))
            } else {
                Err(format!("SID {sid}"))
            }
        }))
    }

    /// Тимчасова тека (прибирається після тесту).
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(name: &str) -> Self {
            let d = std::env::temp_dir()
                .join(format!("optimax-profiles-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Self(d)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ok(r: Option<Reply>) -> serde_json::Value {
        match r {
            Some(Reply::Json(v)) => v,
            other => panic!("{other:?}"),
        }
    }

    fn list(h: &ProfilesHost, sid: &str) -> Vec<GameProfile> {
        serde_json::from_value(ok(h.handle(sid, &Command::ProfilesList))).unwrap()
    }

    #[test]
    fn save_list_remove_per_user() {
        let tmp = Tmp::new("save");
        let h = host(tmp.path());
        let (a, b) = ("S-1-5-21-1-1001", "S-1-5-21-1-1002");
        assert!(list(&h, a).is_empty());
        let mut p = GameProfile::new("TslGame.exe");
        p.game_priority = GamePriority::High;
        let save = Command::ProfileSave {
            profile: serde_json::to_value(&p).unwrap(),
        };
        let _: Vec<serde_json::Value> = serde_json::from_value(ok(h.handle(a, &save))).unwrap();
        assert_eq!(list(&h, a).len(), 1);
        assert_eq!(list(&h, a)[0].exe, "TslGame.exe");
        // Інший користувач свого не бачить.
        assert!(list(&h, b).is_empty());
        assert!(tmp.path().join(a).join(PROFILES_FILE).is_file());
        let remove = |exe: &str| Command::ProfileRemove { exe: exe.into() };
        assert_eq!(
            ok(h.handle(b, &remove("tslgame.exe"))),
            serde_json::json!(false)
        );
        assert_eq!(
            ok(h.handle(a, &remove("tslgame.exe"))),
            serde_json::json!(true)
        );
        assert!(list(&h, a).is_empty());
    }

    #[test]
    fn bad_requests_are_refused() {
        let tmp = Tmp::new("bad");
        let h = host(tmp.path());
        let junk = Command::ProfileSave {
            profile: serde_json::json!({"exe": 5}),
        };
        assert!(matches!(
            h.handle("S-1-5-21-1", &junk),
            Some(Reply::Error(_))
        ));
        // Профіль, який не можна зробити безпечним (exe — не ім'я файлу), не зберігається.
        let bad = Command::ProfileSave {
            profile: serde_json::to_value(GameProfile::new(r"..\cmd.exe")).unwrap(),
        };
        assert!(matches!(
            h.handle("S-1-5-21-1", &bad),
            Some(Reply::Error(_))
        ));
        assert!(list(&h, "S-1-5-21-1").is_empty());
        assert!(matches!(
            h.handle("../x", &Command::ProfilesList),
            Some(Reply::Error(_))
        ));
        assert!(h.handle("S-1-5-21-1", &Command::Sensors).is_none());
    }
}
