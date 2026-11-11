//! Зміни системи в службі (етап 3б з `docs/SERVICE.md`): твіки з каталогу, журнал і відкат.
//!
//! Клієнт не передає ні ключів, ні значень: лише бажаний стан твіку з каталогу
//! ([`optimax_tweaks::Desired`]). План складає й перевіряє рушій служби, запам'ятовує його й
//! повертає номер — застосувати можна лише план із цим номером. Журнал кожного користувача — у
//! його захищеній теці (`%ProgramData%\OptimaX\<SID>`), `HKCU` — його гілка `HKEY_USERS\<SID>`,
//! пріоритет змінюється лише процесам цього самого користувача. Запис автозапуску з теки
//! «Автозавантаження» служба перевіряє сама: файл має лежати в теці цього користувача.

use optimax_service::protocol::{Command, Reply};
use optimax_tweaks::startup::StartupSource;
use optimax_tweaks::{ApplyOutcome, Changes, Confirm, Desired, Engine, Plan, TweakError};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::VecDeque;

/// Скільки складених планів пам'ятаємо (старіші забуваються — їх треба переглянути ще раз).
const PLANS_KEPT: usize = 32;

/// Відповідь на [`Command::TweakPlan`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanReply {
    /// Номер плану в службі.
    pub id: u64,
    /// Що зміниться.
    pub plan: Plan,
}

/// Відповідь на [`Command::TweakApply`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApplyReply {
    /// Застосовано.
    Applied {
        /// Що зроблено.
        outcome: ApplyOutcome,
    },
    /// Точку відновлення не створено — потрібна згода продовжити без неї.
    NeedsConsent {
        /// Чому не створено.
        reason: String,
    },
}

/// Відповідь без діалогу: задана заздалегідь, причина відмови запам'ятовується.
struct Answer {
    allow: bool,
    reason: RefCell<Option<String>>,
}

impl Confirm for Answer {
    fn proceed_without_restore_point(&self, reason: &str) -> bool {
        *self.reason.borrow_mut() = Some(reason.to_string());
        self.allow
    }
}

fn json<T: Serialize>(v: &T) -> Result<Reply, String> {
    serde_json::to_value(v)
        .map(Reply::Json)
        .map_err(|e| e.to_string())
}

/// Чи лежить файл `name` у теці автозавантаження джерела (`pid` — процес клієнта: тека
/// «Автозавантаження» — його користувача).
pub type StartupFileCheck = Box<dyn Fn(u32, StartupSource, &str) -> bool + Send>;

/// Зміни одного користувача: рушій, складені плани, поточна сесія програми.
pub struct UserChanges {
    engine: Engine<'static>,
    plans: VecDeque<(u64, Plan)>,
    next_plan: u64,
    session: Option<u64>,
    startup_files: Option<StartupFileCheck>,
}

impl UserChanges {
    /// Над готовим рушієм (журнал цього користувача, його гілка реєстру).
    pub fn new(engine: Engine<'static>) -> Self {
        Self {
            engine,
            plans: VecDeque::new(),
            next_plan: 1,
            session: None,
            startup_files: None,
        }
    }

    /// Перевірка записів із тек «Автозавантаження» (без неї рушій перевіряє лише, що це ім'я
    /// файлу).
    pub fn with_startup_files(mut self, check: StartupFileCheck) -> Self {
        self.startup_files = Some(check);
        self
    }

    /// Виконує команду змін; `None` — команда не про зміни.
    pub fn handle(&mut self, pid: u32, cmd: &Command) -> Option<Reply> {
        let result = match cmd {
            Command::TweakPlan { desired } => {
                self.prepare(pid).and_then(|()| self.plan(pid, desired))
            }
            Command::TweakApply {
                plan_id,
                allow_without_restore_point,
                session,
            } => self
                .prepare(pid)
                .and_then(|()| self.apply(*plan_id, *allow_without_restore_point, *session)),
            Command::PurgeStandby => self
                .prepare(pid)
                .and_then(|()| self.engine.purge_standby().map_err(|e| e.to_string()))
                .and_then(|id| json(&id)),
            Command::JournalEntries => self
                .prepare(pid)
                .and_then(|()| json(&self.engine.entries())),
            Command::JournalRollback { ids } => self
                .prepare(pid)
                .and_then(|()| json(&self.engine.rollback_ids(ids))),
            Command::JournalRollbackAll => self
                .prepare(pid)
                .and_then(|()| json(&self.engine.rollback_all())),
            Command::JournalRecover => self
                .prepare(pid)
                .and_then(|()| self.engine.recover_pending().map_err(|e| e.to_string()))
                .and_then(|n| json(&n)),
            _ => return None,
        };
        Some(result.unwrap_or_else(Reply::Error))
    }

    /// Перед кожною командою: журнал міг змінити процес без служби — перечитуємо.
    fn prepare(&mut self, pid: u32) -> Result<(), String> {
        self.engine.reload().map_err(|e| e.to_string())?;
        self.engine.set_client(Some(pid));
        Ok(())
    }

    fn plan(&mut self, pid: u32, desired: &serde_json::Value) -> Result<Reply, String> {
        let desired: Desired = serde_json::from_value(desired.clone())
            .map_err(|e| format!("некоректний запит зміни: {e}"))?;
        if let (Desired::StartupApp { source, name, .. }, Some(check)) =
            (&desired, &self.startup_files)
        {
            if source.run_key().is_none() && !check(pid, *source, name) {
                return Err(format!(
                    "{name}: такого файлу в теці автозавантаження немає"
                ));
            }
        }
        let plan = self.engine.plan(&desired).map_err(|e| e.to_string())?;
        let id = self.next_plan;
        self.next_plan += 1;
        if self.plans.len() >= PLANS_KEPT {
            self.plans.pop_front();
        }
        self.plans.push_back((id, plan.clone()));
        json(&PlanReply { id, plan })
    }

    fn apply(&mut self, plan_id: u64, allow: bool, session: u64) -> Result<Reply, String> {
        let Some(pos) = self.plans.iter().position(|(id, _)| *id == plan_id) else {
            return Err(format!("план #{plan_id} — переглянь зміни ще раз"));
        };
        // Новий запуск програми — нова сесія: перед першою зміною знову точка відновлення.
        if self.session != Some(session) {
            self.engine.reset_session();
            self.session = Some(session);
        }
        let plan = self.plans[pos].1.clone();
        let answer = Answer {
            allow,
            reason: RefCell::new(None),
        };
        match Changes::apply(&mut self.engine, &plan, &answer) {
            Ok(outcome) => {
                self.plans.remove(pos);
                tracing::info!(
                    "служба: застосовано план #{plan_id}: {:?}",
                    outcome.entry_ids
                );
                json(&ApplyReply::Applied { outcome })
            }
            Err(TweakError::Cancelled) => match answer.reason.into_inner() {
                Some(reason) => json(&ApplyReply::NeedsConsent { reason }),
                None => Err(TweakError::Cancelled.to_string()),
            },
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(windows)]
pub use self::windows::ChangesHost;

#[cfg(windows)]
mod windows {
    use super::UserChanges;
    use optimax_core::datadir::{data_dir_for, process_user_sid};
    use optimax_core::memory::WinMemory;
    use optimax_core::power::WinPower;
    use optimax_core::process::{PriorityClass, ProcessApi, ProcessInfo, WinProcesses};
    use optimax_core::registry::WinUserRegistry;
    use optimax_core::service::WinServices;
    use optimax_core::sysdirs::{known, known_for_process, Folder};
    use optimax_core::CoreError;
    use optimax_service::host::Peer;
    use optimax_service::protocol::{Command, Reply};
    use optimax_tweaks::restore::WinRestorePoint;
    use optimax_tweaks::startup::{is_plain_file_name, StartupSource};
    use optimax_tweaks::{Engine, FileStore};
    use std::collections::HashMap;
    use std::sync::{Mutex, PoisonError};

    static POWER: WinPower = WinPower;
    static RESTORE: WinRestorePoint = WinRestorePoint;
    static SERVICES: WinServices = WinServices;
    static MEMORY: WinMemory = WinMemory;

    /// Процеси лише цього користувача: служба як SYSTEM могла б змінити пріоритет будь-якого
    /// процесу, а користувач без прав — лише своїх.
    struct OwnedProcesses {
        sid: String,
    }

    impl OwnedProcesses {
        fn check(&self, p: &ProcessInfo) -> Result<(), CoreError> {
            match process_user_sid(p.pid) {
                Ok(sid) if sid == self.sid => Ok(()),
                Ok(_) => Err(CoreError::InvalidData(format!(
                    "{}: процес іншого користувача — пріоритет не змінюємо",
                    p.name
                ))),
                // Процес завершився — хай про це скаже звичайний шлях (NotFound).
                Err(e) => {
                    WinProcesses.priority(p)?;
                    Err(e)
                }
            }
        }
    }

    impl ProcessApi for OwnedProcesses {
        fn find_by_name(&self, name: &str) -> Result<Vec<ProcessInfo>, CoreError> {
            Ok(WinProcesses
                .find_by_name(name)?
                .into_iter()
                .filter(|p| process_user_sid(p.pid).is_ok_and(|s| s == self.sid))
                .collect())
        }

        fn priority(&self, process: &ProcessInfo) -> Result<PriorityClass, CoreError> {
            self.check(process)?;
            WinProcesses.priority(process)
        }

        fn set_priority(
            &self,
            process: &ProcessInfo,
            priority: PriorityClass,
        ) -> Result<(), CoreError> {
            self.check(process)?;
            WinProcesses.set_priority(process, priority)
        }
    }

    /// Рушій для користувача з цим SID. Реєстр, журнал і процеси — свої для кожного SID і
    /// живуть до кінця роботи служби (рушій тримає посилання): створюються раз на користувача.
    fn engine_for(sid: &str) -> Result<Engine<'static>, String> {
        let registry = WinUserRegistry::new(sid).map_err(|e| e.to_string())?;
        let dir = data_dir_for(sid).map_err(|e| e.to_string())?;
        let registry: &'static WinUserRegistry = Box::leak(Box::new(registry));
        let store: &'static FileStore =
            Box::leak(Box::new(FileStore::new(dir.join("journal.json"))));
        let processes: &'static OwnedProcesses =
            Box::leak(Box::new(OwnedProcesses { sid: sid.into() }));
        Engine::new(registry, &POWER, &RESTORE, store)
            .map(|e| {
                e.with_processes(processes)
                    .with_services(&SERVICES)
                    .with_memory(&MEMORY)
            })
            .map_err(|e| e.to_string())
    }

    /// Файл у теці «Автозавантаження»: своя тека — за токеном процесу клієнта (у SYSTEM вона
    /// інша), спільна — одна на всіх.
    fn startup_file_exists(pid: u32, source: StartupSource, name: &str) -> bool {
        let dir = match source {
            StartupSource::UserFolder => known_for_process(Folder::Startup, pid),
            StartupSource::CommonFolder => known(Folder::CommonStartup),
            StartupSource::UserRun | StartupSource::MachineRun | StartupSource::MachineRun32 => {
                return true
            }
        };
        is_plain_file_name(name) && dir.is_some_and(|d| d.join(name).is_file())
    }

    /// Зміни всіх користувачів, що під'єднувались (зазвичай один — той, хто ставив службу).
    #[derive(Default)]
    pub struct ChangesHost {
        users: Mutex<HashMap<String, UserChanges>>,
    }

    impl ChangesHost {
        /// Порожній.
        pub fn new() -> Self {
            Self::default()
        }

        /// Виконує команду змін від клієнта; `None` — команда не про зміни.
        pub fn handle(&self, peer: &Peer, cmd: &Command) -> Option<Reply> {
            let mut users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
            if !users.contains_key(&peer.sid) {
                // Не про зміни — і рушій створювати не треба.
                if !is_change(cmd) {
                    return None;
                }
                match engine_for(&peer.sid) {
                    Ok(e) => {
                        tracing::info!("служба: журнал змін для {}", peer.sid);
                        let user =
                            UserChanges::new(e).with_startup_files(Box::new(startup_file_exists));
                        users.insert(peer.sid.clone(), user);
                    }
                    Err(e) => return Some(Reply::Error(format!("журнал змін недоступний: {e}"))),
                }
            }
            users.get_mut(&peer.sid)?.handle(peer.pid, cmd)
        }
    }

    fn is_change(cmd: &Command) -> bool {
        matches!(
            cmd,
            Command::TweakPlan { .. }
                | Command::TweakApply { .. }
                | Command::PurgeStandby
                | Command::JournalEntries
                | Command::JournalRollback { .. }
                | Command::JournalRollbackAll
                | Command::JournalRecover
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::power::{guids, MemPower};
    use optimax_core::registry::{MemRegistry, RegHive, RegValue, RegistryApi};
    use optimax_tweaks::restore::FakeRestore;
    use optimax_tweaks::{JournalEntry, JournalStore, MemStore, RollbackSummary};

    fn leak<T>(v: T) -> &'static T {
        Box::leak(Box::new(v))
    }

    /// `restore`: `Ok(номер)` або `Err(код помилки)` створення точки відновлення.
    fn host(restore: Result<i64, u32>) -> (UserChanges, &'static MemRegistry, &'static MemStore) {
        let mut reg = MemRegistry::new();
        reg.create_key(RegHive::CurrentUser, optimax_tweaks::engine::GAME_MODE_KEY);
        let reg = leak(reg);
        let power = leak(MemPower::new(guids::BALANCED));
        let store = leak(MemStore::default());
        let engine = Engine::new(reg, power, leak(FakeRestore(restore)), store).unwrap();
        (UserChanges::new(engine), reg, store)
    }

    fn ok(r: Option<Reply>) -> serde_json::Value {
        match r {
            Some(Reply::Json(v)) => v,
            other => panic!("{other:?}"),
        }
    }

    fn plan_game_mode(h: &mut UserChanges, on: bool) -> PlanReply {
        let desired = serde_json::to_value(Desired::GameMode(on)).unwrap();
        serde_json::from_value(ok(h.handle(7, &Command::TweakPlan { desired }))).unwrap()
    }

    #[test]
    fn plan_apply_rollback_through_commands() {
        let (mut h, reg, store) = host(Ok(3));
        let p = plan_game_mode(&mut h, false);
        assert_eq!(p.plan.changes.len(), 1);
        let apply = Command::TweakApply {
            plan_id: p.id,
            allow_without_restore_point: false,
            session: 1,
        };
        let r: ApplyReply = serde_json::from_value(ok(h.handle(7, &apply))).unwrap();
        let ApplyReply::Applied { outcome } = r else {
            panic!("{r:?}")
        };
        assert_eq!(
            reg.read_value(
                RegHive::CurrentUser,
                optimax_tweaks::engine::GAME_MODE_KEY,
                optimax_tweaks::engine::GAME_MODE_VALUE
            )
            .unwrap(),
            Some(RegValue::Dword(0))
        );
        // У журналі — PID клієнта; план застосовано й забуто.
        let entries: Vec<JournalEntry> =
            serde_json::from_value(ok(h.handle(7, &Command::JournalEntries))).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].client_pid, Some(7));
        assert!(matches!(h.handle(7, &apply), Some(Reply::Error(_))));
        let s: RollbackSummary = serde_json::from_value(ok(h.handle(
            7,
            &Command::JournalRollback {
                ids: outcome.entry_ids.clone(),
            },
        )))
        .unwrap();
        assert_eq!(s.rolled_back, outcome.entry_ids);
        assert_eq!(
            reg.read_value(
                RegHive::CurrentUser,
                optimax_tweaks::engine::GAME_MODE_KEY,
                optimax_tweaks::engine::GAME_MODE_VALUE
            )
            .unwrap(),
            None
        );
        assert!(store
            .load()
            .unwrap()
            .iter()
            .all(|e| e.client_pid == Some(7)));
    }

    #[test]
    fn unknown_plan_and_bad_request_are_refused() {
        let (mut h, _, _) = host(Ok(1));
        let apply = Command::TweakApply {
            plan_id: 99,
            allow_without_restore_point: true,
            session: 1,
        };
        assert!(matches!(h.handle(1, &apply), Some(Reply::Error(e)) if e.contains("#99")));
        let junk = Command::TweakPlan {
            desired: serde_json::json!({"tweak": "registry", "value": "HKLM\\SYSTEM"}),
        };
        assert!(matches!(h.handle(1, &junk), Some(Reply::Error(_))));
        assert!(h.handle(1, &Command::Sensors).is_none());
    }

    #[test]
    fn restore_point_consent_and_new_session() {
        let (mut h, _, store) = host(Err(1058));
        let p = plan_game_mode(&mut h, false);
        let apply = |allow, session| Command::TweakApply {
            plan_id: p.id,
            allow_without_restore_point: allow,
            session,
        };
        let r: ApplyReply = serde_json::from_value(ok(h.handle(1, &apply(false, 1)))).unwrap();
        assert!(matches!(r, ApplyReply::NeedsConsent { ref reason } if reason.contains("1058")));
        assert!(store.load().unwrap().is_empty());
        // План зберігся — повтор зі згодою.
        let r: ApplyReply = serde_json::from_value(ok(h.handle(1, &apply(true, 1)))).unwrap();
        assert!(matches!(r, ApplyReply::Applied { .. }));
        // Та сама сесія — точка вже вирішена; нова сесія — питаємо знову.
        let p2 = plan_game_mode(&mut h, true);
        let same = Command::TweakApply {
            plan_id: p2.id,
            allow_without_restore_point: false,
            session: 1,
        };
        let r: ApplyReply = serde_json::from_value(ok(h.handle(1, &same))).unwrap();
        assert!(matches!(r, ApplyReply::Applied { .. }));
        let p3 = plan_game_mode(&mut h, false);
        let next = Command::TweakApply {
            plan_id: p3.id,
            allow_without_restore_point: false,
            session: 2,
        };
        let r: ApplyReply = serde_json::from_value(ok(h.handle(1, &next))).unwrap();
        assert!(matches!(r, ApplyReply::NeedsConsent { .. }));
    }

    #[test]
    fn journal_written_elsewhere_is_reloaded() {
        let (mut h, _, store) = host(Ok(1));
        let p = plan_game_mode(&mut h, false);
        // Програма без служби дописала журнал, поки служба чекала.
        store.save(&[]).unwrap();
        let apply = Command::TweakApply {
            plan_id: p.id,
            allow_without_restore_point: false,
            session: 1,
        };
        assert!(matches!(h.handle(1, &apply), Some(Reply::Json(_))));
        let mut forged = store.load().unwrap();
        forged[0].id = 40;
        store.save(&forged).unwrap();
        let entries: Vec<JournalEntry> =
            serde_json::from_value(ok(h.handle(1, &Command::JournalEntries))).unwrap();
        assert_eq!(entries[0].id, 40);
    }

    #[test]
    fn startup_folder_entry_must_exist_for_the_client() {
        let (h, _, _) = host(Ok(1));
        // Файл є лише в теці клієнта з PID 7.
        let mut h = h.with_startup_files(Box::new(|pid, source, name| {
            pid == 7 && source == StartupSource::UserFolder && name == "Discord.lnk"
        }));
        let plan = |name: &str| Command::TweakPlan {
            desired: serde_json::to_value(Desired::StartupApp {
                source: StartupSource::UserFolder,
                name: name.into(),
                enabled: false,
            })
            .unwrap(),
        };
        let r: PlanReply = serde_json::from_value(ok(h.handle(7, &plan("Discord.lnk")))).unwrap();
        assert_eq!(r.plan.changes.len(), 1);
        for (pid, name) in [(7, "Steam.lnk"), (8, "Discord.lnk")] {
            assert!(
                matches!(h.handle(pid, &plan(name)), Some(Reply::Error(e)) if e.contains("немає")),
                "{pid} {name}"
            );
        }
    }

    #[test]
    fn plans_are_bounded() {
        let (mut h, _, _) = host(Ok(1));
        let first = plan_game_mode(&mut h, false);
        for _ in 0..PLANS_KEPT {
            plan_game_mode(&mut h, false);
        }
        let apply = Command::TweakApply {
            plan_id: first.id,
            allow_without_restore_point: false,
            session: 1,
        };
        assert!(matches!(h.handle(1, &apply), Some(Reply::Error(_))));
    }
}
