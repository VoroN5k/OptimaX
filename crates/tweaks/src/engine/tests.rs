use super::*;
use crate::journal::MemStore;
use optimax_core::power::{guids, MemPower};
use optimax_core::registry::{MemRegistry, RegistryApi};
use optimax_core::CoreError;
use std::cell::Cell;

struct Answer(bool);
impl Confirm for Answer {
    fn proceed_without_restore_point(&self, _reason: &str) -> bool {
        self.0
    }
}

/// Точка відновлення з лічильником викликів.
struct CountingRestore {
    result: Result<i64, u32>,
    calls: Cell<u32>,
}

impl CountingRestore {
    fn ok() -> Self {
        Self {
            result: Ok(7),
            calls: Cell::new(0),
        }
    }
    fn failing() -> Self {
        Self {
            // ERROR_SERVICE_DISABLED: захист системи вимкнено.
            result: Err(1058),
            calls: Cell::new(0),
        }
    }
}

impl RestorePointApi for CountingRestore {
    fn create(&self, _d: &str) -> Result<i64, CoreError> {
        self.calls.set(self.calls.get() + 1);
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
    assert_eq!(restore.calls.get(), 0);
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
    assert_eq!(restore.calls.get(), 1);
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
