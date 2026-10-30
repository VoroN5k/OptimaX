use super::*;
use crate::journal::MemStore;
use optimax_core::power::{guids, MemPower};
use optimax_core::registry::{MemRegistry, RegistryApi};
use optimax_core::CoreError;
use std::sync::atomic::{AtomicU32, Ordering};

struct Answer(bool);
impl Confirm for Answer {
    fn proceed_without_restore_point(&self, _reason: &str) -> bool {
        self.0
    }
}

/// Точка відновлення з лічильником викликів.
struct CountingRestore {
    result: Result<i64, u32>,
    calls: AtomicU32,
}

impl CountingRestore {
    fn ok() -> Self {
        Self {
            result: Ok(7),
            calls: AtomicU32::new(0),
        }
    }
    fn failing() -> Self {
        Self {
            // ERROR_SERVICE_DISABLED: захист системи вимкнено.
            result: Err(1058),
            calls: AtomicU32::new(0),
        }
    }
}

impl RestorePointApi for CountingRestore {
    fn create(&self, _d: &str) -> Result<i64, CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.map_err(|code| CoreError::Os {
            api: "SRSetRestorePointW",
            code,
        })
    }
}

fn registry_with_gamebar(value: Option<RegValue>) -> MemRegistry {
    let mut r = MemRegistry::new();
    r.create_key(RegHive::CurrentUser, GAME_MODE_KEY);
    r.set(
        RegHive::CurrentUser,
        GAME_MODE_KEY,
        "GamepadDoublePressIntervalMs",
        RegValue::Dword(1),
    );
    if let Some(v) = value {
        r.set(RegHive::CurrentUser, GAME_MODE_KEY, GAME_MODE_VALUE, v);
    }
    r
}

fn game_mode(r: &MemRegistry) -> Option<RegValue> {
    r.read_value(RegHive::CurrentUser, GAME_MODE_KEY, GAME_MODE_VALUE)
        .unwrap()
}

#[test]
fn game_mode_apply_then_rollback_restores_absent_value() {
    let reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::HIGH_PERFORMANCE);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();

    let plan = e.plan(&Desired::GameMode(false)).unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].old, SettingValue::Registry(None));
    let out = e.apply(&plan, &Answer(false)).unwrap();
    assert_eq!(
        out.restore_point,
        Some(RestorePointStatus::Created { sequence: 7 })
    );
    assert_eq!(game_mode(&reg), Some(RegValue::Dword(0)));

    // Журнал: запис застосований, із точкою відновлення.
    let j = store.snapshot();
    assert_eq!(j.len(), 1);
    assert_eq!(j[0].state, EntryState::Applied);
    assert_eq!(
        j[0].restore_point,
        Some(RestorePointStatus::Created { sequence: 7 })
    );

    e.rollback(out.entry_ids[0]).unwrap();
    // Стан збігається з початковим: значення знову відсутнє, сусідні значення не зачеплено.
    assert_eq!(game_mode(&reg), None);
    assert_eq!(
        reg.read_value(
            RegHive::CurrentUser,
            GAME_MODE_KEY,
            "GamepadDoublePressIntervalMs"
        )
        .unwrap(),
        Some(RegValue::Dword(1))
    );
    assert!(matches!(
        store.snapshot()[0].state,
        EntryState::RolledBack { .. }
    ));
}

#[test]
fn power_scheme_apply_then_rollback() {
    let reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::HIGH_PERFORMANCE);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();

    let plan = e
        .plan(&Desired::PowerScheme(guids::BALANCED.to_uppercase()))
        .unwrap();
    assert_eq!(
        plan.changes[0].new_display.args["v"],
        format!("Balanced ({})", guids::BALANCED)
    );
    e.apply(&plan, &Answer(false)).unwrap();
    assert_eq!(power.active_scheme().unwrap(), guids::BALANCED);
    let summary = e.rollback_all();
    assert_eq!(summary.failed, vec![]);
    assert_eq!(power.active_scheme().unwrap(), guids::HIGH_PERFORMANCE);
}

#[test]
fn already_in_desired_state_is_noop() {
    let reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    // Відсутнє значення = ігровий режим уже ввімкнено.
    let plan = e.plan(&Desired::GameMode(true)).unwrap();
    assert!(plan.is_noop());
    assert!(e
        .plan(&Desired::PowerScheme(guids::BALANCED.into()))
        .unwrap()
        .is_noop());
    e.apply(&plan, &Answer(false)).unwrap();
    // Порожній план не створює точку відновлення і не пише в журнал.
    assert_eq!(restore.calls.load(Ordering::SeqCst), 0);
    assert!(store.snapshot().is_empty());
}

#[test]
fn wrong_type_is_never_overwritten() {
    let reg = registry_with_gamebar(Some(RegValue::String("1".into())));
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let e = Engine::new(&reg, &power, &restore, &store).unwrap();
    assert!(matches!(
        e.plan(&Desired::GameMode(false)),
        Err(TweakError::TypeMismatch {
            found: "REG_SZ",
            ..
        })
    ));
    assert_eq!(game_mode(&reg), Some(RegValue::String("1".into())));
}

#[test]
fn unknown_power_scheme_rejected() {
    let reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let e = Engine::new(&reg, &power, &restore, &store).unwrap();
    assert!(matches!(
        e.plan(&Desired::PowerScheme(guids::ULTIMATE.into())),
        Err(TweakError::UnknownScheme(_))
    ));
}

#[test]
fn stale_plan_is_refused() {
    let mut reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let plan = {
        let e = Engine::new(&reg, &power, &restore, &store).unwrap();
        e.plan(&Desired::GameMode(false)).unwrap()
    };
    // Хтось змінив значення після перегляду плану.
    reg.set(
        RegHive::CurrentUser,
        GAME_MODE_KEY,
        GAME_MODE_VALUE,
        RegValue::Dword(1),
    );
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    assert!(matches!(
        e.apply(&plan, &Answer(false)),
        Err(TweakError::Stale { .. })
    ));
    assert_eq!(game_mode(&reg), Some(RegValue::Dword(1)));
    assert!(store.snapshot().is_empty());
}

#[test]
fn restore_point_failure_asks_user() {
    let reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::failing();
    let store = MemStore::new();

    // Відмова → нічого не змінено.
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    let plan = e.plan(&Desired::GameMode(false)).unwrap();
    assert!(matches!(
        e.apply(&plan, &Answer(false)),
        Err(TweakError::Cancelled)
    ));
    assert_eq!(game_mode(&reg), None);
    assert!(store.snapshot().is_empty());

    // Згода → зміна застосована, причина записана в журнал.
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    let out = e.apply(&plan, &Answer(true)).unwrap();
    assert!(matches!(
        out.restore_point,
        Some(RestorePointStatus::SkippedByUser { .. })
    ));
    assert!(matches!(
        store.snapshot()[0].restore_point,
        Some(RestorePointStatus::SkippedByUser { .. })
    ));
}

#[test]
fn restore_point_only_once_per_session() {
    let reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::HIGH_PERFORMANCE);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    // Як в A/B: застосувати → відкотити → застосувати → відкотити.
    for _ in 0..3 {
        let plan = e.plan(&Desired::GameMode(false)).unwrap();
        let out = e.apply(&plan, &Answer(false)).unwrap();
        e.rollback(out.entry_ids[0]).unwrap();
    }
    assert_eq!(restore.calls.load(Ordering::SeqCst), 1);
    assert_eq!(game_mode(&reg), None);
    assert_eq!(store.snapshot().len(), 3);
}

#[test]
fn rollback_refuses_to_clobber_external_change() {
    let mut reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let id = {
        let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
        let plan = e.plan(&Desired::GameMode(false)).unwrap();
        e.apply(&plan, &Answer(false)).unwrap().entry_ids[0]
    };
    // Користувач сам змінив налаштування в Параметрах.
    reg.set(
        RegHive::CurrentUser,
        GAME_MODE_KEY,
        GAME_MODE_VALUE,
        RegValue::Dword(1),
    );
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    assert!(matches!(e.rollback(id), Err(TweakError::Conflict { .. })));
    assert_eq!(game_mode(&reg), Some(RegValue::Dword(1)));
    // Запис лишається активним — його видно, і рішення за користувачем.
    assert_eq!(e.active().count(), 1);
}

#[test]
fn rollback_all_restores_everything_newest_first() {
    let reg = registry_with_gamebar(Some(RegValue::Dword(1)));
    let power = MemPower::new(guids::HIGH_PERFORMANCE);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    for d in [
        Desired::PowerScheme(guids::BALANCED.into()),
        Desired::GameMode(false),
    ] {
        let plan = e.plan(&d).unwrap();
        e.apply(&plan, &Answer(false)).unwrap();
    }
    let s = e.rollback_all();
    assert_eq!(s.rolled_back, vec![2, 1]);
    assert_eq!(game_mode(&reg), Some(RegValue::Dword(1)));
    assert_eq!(power.active_scheme().unwrap(), guids::HIGH_PERFORMANCE);
    assert_eq!(e.active().count(), 0);
}

/// Реєстр, який «приймає» запис, але нічого не змінює (як заблокована політикою гілка).
struct IgnoringRegistry(MemRegistry);

impl RegistryApi for IgnoringRegistry {
    fn read_value(&self, h: RegHive, p: &str, n: &str) -> Result<Option<RegValue>, CoreError> {
        self.0.read_value(h, p, n)
    }
    fn value_names(&self, h: RegHive, p: &str) -> Result<Vec<String>, CoreError> {
        self.0.value_names(h, p)
    }
}

impl RegistryWriteApi for IgnoringRegistry {
    fn write_value(&self, _h: RegHive, _p: &str, _n: &str, _v: &RegValue) -> Result<(), CoreError> {
        Ok(())
    }
    fn delete_value(&self, _h: RegHive, _p: &str, _n: &str) -> Result<(), CoreError> {
        Ok(())
    }
}

#[test]
fn unverified_write_is_reported_and_journaled_as_failed() {
    let reg = IgnoringRegistry(registry_with_gamebar(None));
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    let plan = e.plan(&Desired::GameMode(false)).unwrap();
    assert!(matches!(
        e.apply(&plan, &Answer(false)),
        Err(TweakError::VerifyFailed { .. })
    ));
    assert!(matches!(
        store.snapshot()[0].state,
        EntryState::Failed { .. }
    ));
    assert_eq!(e.active().count(), 0);
}

#[test]
fn pending_entry_after_crash_is_rolled_back() {
    let reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    // Імітуємо збій: запис Pending, а зміна в системі вже відбулась.
    store
        .save(&[JournalEntry {
            id: 1,
            tweak_id: "game_mode".into(),
            target: SettingTarget::Registry {
                hive: RegHive::CurrentUser,
                path: GAME_MODE_KEY.into(),
                name: GAME_MODE_VALUE.into(),
            },
            old: SettingValue::Registry(None),
            new: SettingValue::Registry(Some(RegValue::Dword(0))),
            created_unix: 0,
            state: EntryState::Pending,
            restore_point: None,
        }])
        .unwrap();
    reg.write_value(
        RegHive::CurrentUser,
        GAME_MODE_KEY,
        GAME_MODE_VALUE,
        &RegValue::Dword(0),
    )
    .unwrap();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    assert_eq!(e.recover_pending().unwrap(), 1);
    assert_eq!(game_mode(&reg), None);
    assert!(matches!(
        store.snapshot()[0].state,
        EntryState::RolledBack { .. }
    ));
    // Повторний виклик нічого не робить.
    assert_eq!(e.recover_pending().unwrap(), 0);
}

#[test]
fn pending_entry_that_never_happened_is_marked_failed() {
    let reg = registry_with_gamebar(None);
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    store
        .save(&[JournalEntry {
            id: 1,
            tweak_id: "power_scheme".into(),
            target: SettingTarget::PowerScheme,
            old: SettingValue::PowerScheme(guids::BALANCED.into()),
            new: SettingValue::PowerScheme(guids::HIGH_PERFORMANCE.into()),
            created_unix: 0,
            state: EntryState::Pending,
            restore_point: None,
        }])
        .unwrap();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    e.recover_pending().unwrap();
    assert_eq!(power.active_scheme().unwrap(), guids::BALANCED);
    assert!(matches!(
        store.snapshot()[0].state,
        EntryState::Failed { .. }
    ));
}

mod gamemode_targets {
    use super::*;
    use crate::TweakId;
    use optimax_core::process::{MemProcesses, PriorityClass, ProcessApi};
    use optimax_core::service::{MemServices, ServiceApi, ServiceState};

    #[test]
    fn process_priority_apply_rollback_and_exit() {
        let reg = registry_with_gamebar(None);
        let power = MemPower::new(guids::BALANCED);
        let restore = CountingRestore::ok();
        let store = MemStore::new();
        let procs = MemProcesses::new();
        let game = procs.spawn(100, "game.exe", 1);
        let chrome = procs.spawn(200, "chrome.exe", 2);
        let mut e = Engine::new(&reg, &power, &restore, &store)
            .unwrap()
            .with_processes(&procs);

        let up = e
            .plan(&Desired::ProcessPriority {
                process: game.clone(),
                priority: PriorityClass::AboveNormal,
            })
            .unwrap();
        let down = e
            .plan(&Desired::ProcessPriority {
                process: chrome.clone(),
                priority: PriorityClass::BelowNormal,
            })
            .unwrap();
        let a = e.apply(&up, &Answer(false)).unwrap().entry_ids[0];
        let b = e.apply(&down, &Answer(false)).unwrap().entry_ids[0];
        assert_eq!(procs.priority(&game).unwrap(), PriorityClass::AboveNormal);
        assert_eq!(procs.priority(&chrome).unwrap(), PriorityClass::BelowNormal);

        // Гру закрили до відкату — це не помилка, просто нічого відновлювати.
        procs.kill(100);
        let s = e.rollback_ids(&[a, b]);
        assert_eq!(s.failed, vec![]);
        assert_eq!(s.rolled_back, vec![b, a]);
        assert_eq!(procs.priority(&chrome).unwrap(), PriorityClass::Normal);
    }

    #[test]
    fn protected_and_realtime_are_forbidden() {
        let reg = registry_with_gamebar(None);
        let power = MemPower::new(guids::BALANCED);
        let restore = CountingRestore::ok();
        let store = MemStore::new();
        let procs = MemProcesses::new();
        let audio = procs.spawn(5, "audiodg.exe", 1);
        let game = procs.spawn(6, "game.exe", 1);
        let e = Engine::new(&reg, &power, &restore, &store)
            .unwrap()
            .with_processes(&procs);
        assert!(matches!(
            e.plan(&Desired::ProcessPriority {
                process: audio,
                priority: PriorityClass::BelowNormal
            }),
            Err(TweakError::Forbidden(_))
        ));
        assert!(matches!(
            e.plan(&Desired::ProcessPriority {
                process: game,
                priority: PriorityClass::Realtime
            }),
            Err(TweakError::Forbidden(_))
        ));
    }

    #[test]
    fn service_stop_and_restart() {
        let reg = registry_with_gamebar(None);
        let power = MemPower::new(guids::BALANCED);
        let restore = CountingRestore::ok();
        let store = MemStore::new();
        let svc = MemServices::new(&[
            ("WSearch", ServiceState::Running),
            ("SysMain", ServiceState::Stopped),
            ("wuauserv", ServiceState::Running),
        ]);
        let mut e = Engine::new(&reg, &power, &restore, &store)
            .unwrap()
            .with_services(&svc);
        let stop = |name: &str| Desired::Service {
            name: name.into(),
            state: ServiceState::Stopped,
        };
        let plan = e.plan(&stop("WSearch")).unwrap();
        let id = e.apply(&plan, &Answer(false)).unwrap().entry_ids[0];
        assert_eq!(svc.state("WSearch").unwrap(), ServiceState::Stopped);
        // Уже зупинена служба — нічого не робимо (і не запустимо її потім).
        assert!(e.plan(&stop("SysMain")).unwrap().is_noop());
        // Оновлення Windows не чіпаємо ніколи.
        assert!(matches!(
            e.plan(&stop("wuauserv")),
            Err(TweakError::Forbidden(_))
        ));
        e.rollback(id).unwrap();
        assert_eq!(svc.state("WSearch").unwrap(), ServiceState::Running);
    }

    #[test]
    fn missing_platform_api_is_reported() {
        let reg = registry_with_gamebar(None);
        let power = MemPower::new(guids::BALANCED);
        let restore = CountingRestore::ok();
        let store = MemStore::new();
        let e = Engine::new(&reg, &power, &restore, &store).unwrap();
        assert!(matches!(
            e.plan(&Desired::Service {
                name: "WSearch".into(),
                state: ServiceState::Stopped
            }),
            Err(TweakError::Unavailable(_))
        ));
    }

    #[test]
    fn one_shot_action_is_journaled_but_not_rolled_back() {
        let reg = registry_with_gamebar(None);
        let power = MemPower::new(guids::HIGH_PERFORMANCE);
        let restore = CountingRestore::ok();
        let store = MemStore::new();
        let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
        // Зміна користувача поза сесією гри — rollback_ids сесії її не зачепить.
        let user = e
            .plan(&Desired::PowerScheme(guids::BALANCED.into()))
            .unwrap();
        e.apply(&user, &Answer(false)).unwrap();

        let id = e
            .perform(TweakId::StandbyPurge, SettingTarget::StandbyList, || Ok(()))
            .unwrap();
        assert_eq!(
            store.snapshot().last().unwrap().state,
            EntryState::Performed
        );
        assert!(e.active().all(|x| x.id != id));
        let failed = e.perform(TweakId::StandbyPurge, SettingTarget::StandbyList, || {
            Err(CoreError::Os {
                api: "NtSetSystemInformation",
                code: 5,
            })
        });
        assert!(failed.is_err());
        assert!(matches!(
            store.snapshot().last().unwrap().state,
            EntryState::Failed { .. }
        ));

        let s = e.rollback_ids(&[id]);
        assert!(s.rolled_back.is_empty());
        assert_eq!(power.active_scheme().unwrap(), guids::BALANCED);
    }
}

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn startup_registry() -> MemRegistry {
    let mut r = MemRegistry::new();
    r.set(
        RegHive::CurrentUser,
        RUN,
        "Discord",
        RegValue::String(r#""C:\Users\u\Discord\Update.exe" --processStart Discord.exe"#.into()),
    );
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\WINDOWS".into());
    r.set(
        RegHive::LocalMachine,
        RUN,
        "SecurityHealth",
        RegValue::String(format!(r"{system_root}\system32\SecurityHealthSystray.exe")),
    );
    r.create_key(
        RegHive::CurrentUser,
        &crate::startup::StartupSource::UserRun.approved_key(),
    );
    r
}

fn startup(source: crate::startup::StartupSource, name: &str, enabled: bool) -> Desired {
    Desired::StartupApp {
        source,
        name: name.into(),
        enabled,
    }
}

#[test]
fn startup_disable_then_rollback_restores_absent_flag() {
    use crate::startup::StartupSource;
    let reg = startup_registry();
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
    let key = StartupSource::UserRun.approved_key();

    let plan = e
        .plan(&startup(StartupSource::UserRun, "Discord", false))
        .unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].old, SettingValue::Registry(None));
    assert_eq!(
        plan.changes[0].new_display.key,
        "tweak.startup_app.disabled"
    );
    let out = e.apply(&plan, &Answer(false)).unwrap();
    let flag = reg
        .read_value(RegHive::CurrentUser, &key, "Discord")
        .unwrap();
    assert_eq!(crate::startup::is_enabled(flag.as_ref()), Ok(false));
    // Сам запис у Run не змінено.
    assert!(matches!(
        reg.read_value(RegHive::CurrentUser, RUN, "Discord")
            .unwrap(),
        Some(RegValue::String(_))
    ));
    // Уже вимкнено — нічого планувати.
    assert!(e
        .plan(&startup(StartupSource::UserRun, "Discord", false))
        .unwrap()
        .is_noop());

    e.rollback(out.entry_ids[0]).unwrap();
    assert_eq!(
        reg.read_value(RegHive::CurrentUser, &key, "Discord")
            .unwrap(),
        None
    );
}

#[test]
fn startup_refuses_system_missing_and_odd_entries() {
    use crate::startup::StartupSource;
    let mut reg = startup_registry();
    reg.set(
        RegHive::CurrentUser,
        &StartupSource::UserRun.approved_key(),
        "Discord",
        RegValue::Dword(3),
    );
    let power = MemPower::new(guids::BALANCED);
    let restore = CountingRestore::ok();
    let store = MemStore::new();
    let e = Engine::new(&reg, &power, &restore, &store).unwrap();
    let forbidden = |d: Desired| matches!(e.plan(&d), Err(TweakError::Forbidden(_)));
    assert!(forbidden(startup(
        StartupSource::MachineRun,
        "SecurityHealth",
        false
    )));
    assert!(forbidden(startup(StartupSource::UserRun, "Nope", false)));
    assert!(forbidden(startup(
        StartupSource::UserFolder,
        r"..\evil.lnk",
        false
    )));
    // Прапорець не REG_BINARY — не перезаписуємо.
    assert!(matches!(
        e.plan(&startup(StartupSource::UserRun, "Discord", false)),
        Err(TweakError::TypeMismatch {
            expected: "REG_BINARY",
            ..
        })
    ));
}

/// Журнал лежить у теці, куди може писати процес без прав адміністратора. Підроблений запис
/// не повинен змусити OptimaX (з правами адміністратора) записати довільне значення в реєстр.
mod forged_journal {
    use super::*;

    const IFEO: &str =
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\sethc.exe";

    fn forged(state: EntryState) -> JournalEntry {
        JournalEntry {
            id: 1,
            tweak_id: "game_mode".into(),
            target: SettingTarget::Registry {
                hive: RegHive::LocalMachine,
                path: IFEO.into(),
                name: "Debugger".into(),
            },
            old: SettingValue::Registry(Some(RegValue::String(r"C:\Users\Test\evil.exe".into()))),
            new: SettingValue::Registry(None),
            created_unix: 0,
            state,
            restore_point: None,
        }
    }

    fn setup(state: EntryState) -> (MemRegistry, MemStore) {
        let mut reg = registry_with_gamebar(None);
        // Ключ існує — запис у нього вдався б, тож тест не пройде «випадково».
        reg.create_key(RegHive::LocalMachine, IFEO);
        let store = MemStore::new();
        store.save(&[forged(state)]).unwrap();
        (reg, store)
    }

    fn debugger(reg: &MemRegistry) -> Option<RegValue> {
        reg.read_value(RegHive::LocalMachine, IFEO, "Debugger")
            .unwrap()
    }

    #[test]
    fn pending_entry_with_foreign_target_is_not_written_on_startup() {
        let (reg, store) = setup(EntryState::Pending);
        let power = MemPower::new(guids::BALANCED);
        let restore = CountingRestore::ok();
        let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
        let _ = e.recover_pending();
        assert_eq!(debugger(&reg), None);
        assert!(!matches!(
            store.snapshot()[0].state,
            EntryState::RolledBack { .. }
        ));
    }

    #[test]
    fn applied_entry_with_foreign_target_is_not_rolled_back() {
        let (reg, store) = setup(EntryState::Applied);
        let power = MemPower::new(guids::BALANCED);
        let restore = CountingRestore::ok();
        let mut e = Engine::new(&reg, &power, &restore, &store).unwrap();
        assert!(e.rollback(1).is_err());
        assert_eq!(debugger(&reg), None);
    }

    fn entry(
        tweak: &str,
        target: SettingTarget,
        old: SettingValue,
        new: SettingValue,
    ) -> JournalEntry {
        JournalEntry {
            tweak_id: tweak.into(),
            target,
            old,
            new,
            ..forged(EntryState::Applied)
        }
    }

    fn reg(hive: RegHive, path: &str, name: &str) -> SettingTarget {
        SettingTarget::Registry {
            hive,
            path: path.into(),
            name: name.into(),
        }
    }

    #[test]
    fn genuine_targets_of_every_tweak_are_accepted() {
        use crate::startup::StartupSource;
        use optimax_core::process::{PriorityClass, ProcessInfo};
        use optimax_core::service::ServiceState;
        let none = || SettingValue::Registry(None);
        let ok = [
            entry(
                "game_mode",
                reg(RegHive::CurrentUser, GAME_MODE_KEY, GAME_MODE_VALUE),
                none(),
                SettingValue::Registry(Some(RegValue::Dword(1))),
            ),
            entry(
                "gpu_preference",
                reg(RegHive::CurrentUser, GPU_PREF_KEY, r"D:\Games\game.exe"),
                none(),
                SettingValue::Registry(Some(RegValue::String("GpuPreference=2;".into()))),
            ),
            entry(
                "startup_app",
                reg(
                    RegHive::LocalMachine,
                    &StartupSource::MachineRun32.approved_key(),
                    "Steam",
                ),
                none(),
                SettingValue::Registry(Some(RegValue::Binary(vec![3; 12]))),
            ),
            entry(
                "power_scheme",
                SettingTarget::PowerScheme,
                SettingValue::PowerScheme(guids::BALANCED.into()),
                SettingValue::PowerScheme(guids::HIGH_PERFORMANCE.into()),
            ),
            entry(
                "process_priority",
                SettingTarget::ProcessPriority {
                    process: ProcessInfo {
                        pid: 42,
                        name: "chrome.exe".into(),
                        start_time: 1,
                    },
                },
                SettingValue::Priority(PriorityClass::Normal),
                SettingValue::Priority(PriorityClass::BelowNormal),
            ),
            entry(
                "service",
                SettingTarget::Service {
                    name: "WSearch".into(),
                },
                SettingValue::Service(ServiceState::Running),
                SettingValue::Service(ServiceState::Stopped),
            ),
        ];
        for e in &ok {
            assert!(is_known_target(e), "{}", e.target.location());
        }
    }

    #[test]
    fn mismatched_tweak_target_or_type_is_rejected() {
        let bad = [
            // Ключ ігрового режиму, але інше значення.
            entry(
                "game_mode",
                reg(RegHive::CurrentUser, GAME_MODE_KEY, "Debugger"),
                SettingValue::Registry(None),
                SettingValue::Registry(Some(RegValue::Dword(1))),
            ),
            // Правильний ключ, але рядок замість DWORD.
            entry(
                "game_mode",
                reg(RegHive::CurrentUser, GAME_MODE_KEY, GAME_MODE_VALUE),
                SettingValue::Registry(Some(RegValue::String("x".into()))),
                SettingValue::Registry(None),
            ),
            // Ціль іншого твіку під чужим ідентифікатором.
            entry(
                "power_scheme",
                SettingTarget::Service {
                    name: "WSearch".into(),
                },
                SettingValue::Service(optimax_core::service::ServiceState::Running),
                SettingValue::Service(optimax_core::service::ServiceState::Stopped),
            ),
            // Служба поза списком дозволених.
            entry(
                "service",
                SettingTarget::Service {
                    name: "MpsSvc".into(),
                },
                SettingValue::Service(optimax_core::service::ServiceState::Stopped),
                SettingValue::Service(optimax_core::service::ServiceState::Running),
            ),
            entry(
                "unknown_tweak",
                SettingTarget::PowerScheme,
                SettingValue::PowerScheme(guids::BALANCED.into()),
                SettingValue::PowerScheme(guids::BALANCED.into()),
            ),
        ];
        for e in &bad {
            assert!(!is_known_target(e), "{}", e.target.location());
        }
    }
}
