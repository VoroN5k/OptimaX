//! Ігрова сесія: застосувати профіль на старті гри, повернути все після виходу.

use crate::profile::GameProfile;
use optimax_core::memory::MemoryApi;
use optimax_core::process::{PriorityClass, ProcessApi, ProcessInfo};
use optimax_core::service::ServiceState;
use optimax_core::Text;
use optimax_tweaks::engine::RollbackSummary;
use optimax_tweaks::{
    Confirm, Desired, Engine, EntryState, RestorePointStatus, SettingTarget, TweakError, TweakId,
};

/// Що зроблено на старті сесії.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionReport {
    /// Записи журналу, які треба відкотити після гри.
    pub entry_ids: Vec<u64>,
    /// Що застосовано (для показу користувачу).
    pub applied: Vec<Text>,
    /// Що пропущено й чому.
    pub skipped: Vec<Text>,
    /// Точка відновлення, якщо створена на цьому старті.
    pub restore_point: Option<RestorePointStatus>,
}

fn rank(p: PriorityClass) -> u8 {
    match p {
        PriorityClass::Idle => 0,
        PriorityClass::BelowNormal => 1,
        PriorityClass::Normal => 2,
        PriorityClass::AboveNormal => 3,
        PriorityClass::High => 4,
        PriorityClass::Realtime => 5,
    }
}

/// Застосовує профіль до запущеної гри. Помилка окремої зміни не зупиняє решту
/// (її видно в `skipped`); відмова користувача без точки відновлення — скасовує все.
pub fn start_session(
    engine: &mut Engine<'_>,
    processes: &dyn ProcessApi,
    memory: &dyn MemoryApi,
    profile: &GameProfile,
    game: &ProcessInfo,
    confirm: &dyn Confirm,
) -> Result<SessionReport, TweakError> {
    let (profile, warnings) = profile
        .sanitized()
        .map_err(|e| TweakError::Forbidden(e.to_string()))?;
    let mut report = SessionReport {
        skipped: warnings,
        ..SessionReport::default()
    };

    let mut wanted: Vec<(Desired, Text)> = Vec::new();
    if let Some(target) = profile.game_priority.class() {
        match processes.priority(game) {
            // Не знижуємо грі пріоритет, якщо вона сама (чи користувач) поставила вищий.
            Ok(current) if rank(current) >= rank(target) => {
                report
                    .skipped
                    .push(Text::new("gm.skip.game_already_high").arg("name", &game.name));
            }
            Ok(_) => wanted.push((
                Desired::ProcessPriority {
                    process: game.clone(),
                    priority: target,
                },
                Text::new(if target == PriorityClass::High {
                    "gm.applied.game_high"
                } else {
                    "gm.applied.game_above_normal"
                })
                .arg("name", &game.name),
            )),
            Err(e) => report.skipped.push(
                Text::new("gm.skip.error")
                    .arg("what", &game.name)
                    .arg("error", e),
            ),
        }
    }
    for name in &profile.lower_background {
        let found = match processes.find_by_name(name) {
            Ok(f) => f,
            Err(e) => {
                report
                    .skipped
                    .push(Text::new("gm.skip.error").arg("what", name).arg("error", e));
                continue;
            }
        };
        if found.is_empty() {
            report
                .skipped
                .push(Text::new("gm.skip.not_running").arg("name", name));
        }
        for p in found.into_iter().filter(|p| p.pid != game.pid) {
            // Пріоритет, який уже нижчий (поставив користувач), не чіпаємо.
            match processes.priority(&p) {
                Ok(current) if rank(current) <= rank(PriorityClass::BelowNormal) => {}
                Ok(_) => wanted.push((
                    Desired::ProcessPriority {
                        process: p.clone(),
                        priority: PriorityClass::BelowNormal,
                    },
                    Text::new("gm.applied.background")
                        .arg("name", &p.name)
                        .arg("pid", p.pid),
                )),
                Err(_) => {}
            }
        }
    }
    for name in &profile.stop_services {
        wanted.push((
            Desired::Service {
                name: name.clone(),
                state: ServiceState::Stopped,
            },
            Text::new("gm.applied.service").arg("name", name),
        ));
    }

    for (desired, label) in wanted {
        let plan = match engine.plan(&desired) {
            Ok(p) => p,
            Err(e) => {
                report.skipped.push(
                    Text::new("gm.skip.error")
                        .arg("what", label_target(&desired))
                        .arg("error", e),
                );
                continue;
            }
        };
        if plan.is_noop() {
            if let Desired::Service { name, .. } = &desired {
                report
                    .skipped
                    .push(Text::new("gm.skip.service_not_running").arg("name", name));
            }
            continue;
        }
        match engine.apply(&plan, confirm) {
            Ok(out) => {
                if out.restore_point.is_some() {
                    report.restore_point = out.restore_point;
                }
                report.entry_ids.extend(out.entry_ids);
                report.applied.push(label);
            }
            Err(TweakError::Cancelled) => {
                // Без згоди користувача нічого не лишаємо.
                engine.rollback_ids(&report.entry_ids);
                return Err(TweakError::Cancelled);
            }
            Err(e) => report.skipped.push(
                Text::new("gm.skip.error")
                    .arg("what", label_target(&desired))
                    .arg("error", e),
            ),
        }
    }

    if profile.purge_standby_when_low {
        match memory.status() {
            Ok(status) if status.is_low() => {
                let mib = status.available_bytes / (1024 * 1024);
                match engine.perform(TweakId::StandbyPurge, SettingTarget::StandbyList, || {
                    memory.purge_standby()
                }) {
                    Ok(_) => report
                        .applied
                        .push(Text::new("gm.applied.standby").arg("free", mib)),
                    Err(e) => report.skipped.push(
                        Text::new("gm.skip.error")
                            .arg("what", "standby")
                            .arg("error", e),
                    ),
                }
            }
            Ok(status) => report.skipped.push(
                Text::new("gm.skip.memory_ok").arg("free", status.available_bytes / (1024 * 1024)),
            ),
            Err(e) => report.skipped.push(
                Text::new("gm.skip.error")
                    .arg("what", "RAM")
                    .arg("error", e),
            ),
        }
    }
    Ok(report)
}

fn label_target(d: &Desired) -> String {
    match d {
        Desired::ProcessPriority { process, .. } => format!("{} ({})", process.name, process.pid),
        Desired::Service { name, .. } => name.clone(),
        other => format!("{other:?}"),
    }
}

/// Повертає все, що змінила сесія.
pub fn end_session(engine: &mut Engine<'_>, report: &SessionReport) -> RollbackSummary {
    engine.rollback_ids(&report.entry_ids)
}

/// Після аварійного завершення: повертає служби й пріоритети, що лишилися від режиму гри.
/// Інші зміни користувача (схема живлення тощо) не чіпає.
pub fn restore_leftovers(engine: &mut Engine<'_>) -> RollbackSummary {
    let ids: Vec<u64> = engine
        .entries()
        .iter()
        .filter(|e| {
            e.state == EntryState::Applied
                && (e.tweak_id == TweakId::ProcessPriority.as_str()
                    || e.tweak_id == TweakId::Service.as_str())
        })
        .map(|e| e.id)
        .collect();
    engine.rollback_ids(&ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::GamePriority;
    use optimax_core::memory::FakeMemory;
    use optimax_core::power::{guids, MemPower, PowerApi};
    use optimax_core::process::MemProcesses;
    use optimax_core::registry::MemRegistry;
    use optimax_core::service::{MemServices, ServiceApi};
    use optimax_tweaks::restore::FakeRestore;
    use optimax_tweaks::MemStore;

    const GIB: u64 = 1024 * 1024 * 1024;

    struct Yes;
    impl Confirm for Yes {
        fn proceed_without_restore_point(&self, _r: &str) -> bool {
            true
        }
    }
    struct No;
    impl Confirm for No {
        fn proceed_without_restore_point(&self, _r: &str) -> bool {
            false
        }
    }

    struct World {
        reg: MemRegistry,
        power: MemPower,
        store: MemStore,
        procs: MemProcesses,
        svc: MemServices,
    }

    fn world() -> World {
        World {
            reg: MemRegistry::new(),
            power: MemPower::new(guids::HIGH_PERFORMANCE),
            store: MemStore::new(),
            procs: MemProcesses::new(),
            svc: MemServices::new(&[
                ("WSearch", ServiceState::Running),
                ("SysMain", ServiceState::Stopped),
            ]),
        }
    }

    fn full_profile() -> GameProfile {
        GameProfile {
            exe: "game.exe".into(),
            game_priority: GamePriority::AboveNormal,
            lower_background: vec![
                "chrome.exe".into(),
                "notrunning.exe".into(),
                "audiodg.exe".into(),
            ],
            stop_services: vec!["WSearch".into(), "SysMain".into()],
            purge_standby_when_low: true,
        }
    }

    #[test]
    fn full_session_applies_and_restores_everything() {
        let w = world();
        let restore = FakeRestore(Ok(3));
        let mem = FakeMemory::new(16 * GIB, GIB / 2);
        let game = w.procs.spawn(10, "game.exe", 1);
        let c1 = w.procs.spawn(20, "chrome.exe", 2);
        let c2 = w.procs.spawn(21, "chrome.exe", 3);
        let audio = w.procs.spawn(30, "audiodg.exe", 4);
        let mut engine = Engine::new(&w.reg, &w.power, &restore, &w.store)
            .unwrap()
            .with_processes(&w.procs)
            .with_services(&w.svc);

        let report =
            start_session(&mut engine, &w.procs, &mem, &full_profile(), &game, &Yes).unwrap();
        assert_eq!(
            report.restore_point,
            Some(RestorePointStatus::Created { sequence: 3 })
        );
        assert_eq!(w.procs.priority(&game).unwrap(), PriorityClass::AboveNormal);
        assert_eq!(w.procs.priority(&c1).unwrap(), PriorityClass::BelowNormal);
        assert_eq!(w.procs.priority(&c2).unwrap(), PriorityClass::BelowNormal);
        assert_eq!(w.procs.priority(&audio).unwrap(), PriorityClass::Normal);
        assert_eq!(w.svc.state("WSearch").unwrap(), ServiceState::Stopped);
        assert_eq!(mem.purges(), 1);
        // Гра + 2 chrome + WSearch. Очищення standby — разова дія, не відкочується.
        assert_eq!(report.entry_ids.len(), 4);
        let skipped: Vec<&str> = report.skipped.iter().map(|t| t.key.as_str()).collect();
        assert!(skipped.contains(&"gm.warn.protected"));
        assert!(skipped.contains(&"gm.skip.not_running"));
        assert!(skipped.contains(&"gm.skip.service_not_running"));

        // Один chrome закрили під час гри — відкат не має падати.
        w.procs.kill(21);
        let summary = end_session(&mut engine, &report);
        assert_eq!(summary.failed, vec![]);
        assert_eq!(w.procs.priority(&game).unwrap(), PriorityClass::Normal);
        assert_eq!(w.procs.priority(&c1).unwrap(), PriorityClass::Normal);
        assert_eq!(w.svc.state("WSearch").unwrap(), ServiceState::Running);
        assert_eq!(w.svc.state("SysMain").unwrap(), ServiceState::Stopped);
        assert_eq!(engine.active().count(), 0);
    }

    #[test]
    fn enough_memory_skips_purge() {
        let w = world();
        let restore = FakeRestore(Ok(1));
        let mem = FakeMemory::new(32 * GIB, 20 * GIB);
        let game = w.procs.spawn(10, "game.exe", 1);
        let mut engine = Engine::new(&w.reg, &w.power, &restore, &w.store)
            .unwrap()
            .with_processes(&w.procs)
            .with_services(&w.svc);
        let mut p = GameProfile::new("game.exe");
        p.purge_standby_when_low = true;
        let r = start_session(&mut engine, &w.procs, &mem, &p, &game, &Yes).unwrap();
        assert_eq!(mem.purges(), 0);
        assert!(r.skipped.iter().any(|t| t.key == "gm.skip.memory_ok"));
    }

    #[test]
    fn game_already_high_is_not_lowered() {
        let w = world();
        let restore = FakeRestore(Ok(1));
        let mem = FakeMemory::new(32 * GIB, 20 * GIB);
        let game = w.procs.spawn(10, "game.exe", 1);
        w.procs.set_priority(&game, PriorityClass::High).unwrap();
        let mut engine = Engine::new(&w.reg, &w.power, &restore, &w.store)
            .unwrap()
            .with_processes(&w.procs);
        let r = start_session(
            &mut engine,
            &w.procs,
            &mem,
            &GameProfile::new("game.exe"),
            &game,
            &Yes,
        )
        .unwrap();
        assert!(r.entry_ids.is_empty());
        assert_eq!(w.procs.priority(&game).unwrap(), PriorityClass::High);
    }

    #[test]
    fn refused_without_restore_point_changes_nothing() {
        let w = world();
        let restore = FakeRestore(Err(1058));
        let mem = FakeMemory::new(16 * GIB, GIB / 2);
        let game = w.procs.spawn(10, "game.exe", 1);
        let chrome = w.procs.spawn(20, "chrome.exe", 2);
        let mut engine = Engine::new(&w.reg, &w.power, &restore, &w.store)
            .unwrap()
            .with_processes(&w.procs)
            .with_services(&w.svc);
        let r = start_session(&mut engine, &w.procs, &mem, &full_profile(), &game, &No);
        assert!(matches!(r, Err(TweakError::Cancelled)));
        assert_eq!(w.procs.priority(&game).unwrap(), PriorityClass::Normal);
        assert_eq!(w.procs.priority(&chrome).unwrap(), PriorityClass::Normal);
        assert_eq!(w.svc.state("WSearch").unwrap(), ServiceState::Running);
        assert_eq!(mem.purges(), 0);
    }

    #[test]
    fn leftovers_after_crash_are_restored_but_user_tweaks_kept() {
        let w = world();
        let restore = FakeRestore(Ok(1));
        let mem = FakeMemory::new(32 * GIB, 20 * GIB);
        let game = w.procs.spawn(10, "game.exe", 1);
        {
            let mut engine = Engine::new(&w.reg, &w.power, &restore, &w.store)
                .unwrap()
                .with_processes(&w.procs)
                .with_services(&w.svc);
            // Зміна користувача, зроблена окремо від режиму гри.
            let plan = engine
                .plan(&Desired::PowerScheme(guids::BALANCED.into()))
                .unwrap();
            engine.apply(&plan, &Yes).unwrap();
            let mut p = GameProfile::new("game.exe");
            p.stop_services = vec!["WSearch".into()];
            start_session(&mut engine, &w.procs, &mem, &p, &game, &Yes).unwrap();
            // «Аварія»: end_session не викликано.
        }
        let mut engine = Engine::new(&w.reg, &w.power, &restore, &w.store)
            .unwrap()
            .with_processes(&w.procs)
            .with_services(&w.svc);
        let s = restore_leftovers(&mut engine);
        assert_eq!(s.rolled_back.len(), 2);
        assert_eq!(w.svc.state("WSearch").unwrap(), ServiceState::Running);
        assert_eq!(w.procs.priority(&game).unwrap(), PriorityClass::Normal);
        assert_eq!(w.power.active_scheme().unwrap(), guids::BALANCED);
    }
}

#[cfg(test)]
mod localization {
    use super::*;
    use crate::profile::GamePriority;
    use optimax_core::memory::FakeMemory;
    use optimax_core::power::{guids, MemPower};
    use optimax_core::process::MemProcesses;
    use optimax_core::registry::MemRegistry;
    use optimax_core::service::MemServices;
    use optimax_core::Catalog;
    use optimax_tweaks::restore::FakeRestore;
    use optimax_tweaks::MemStore;

    struct Yes;
    impl Confirm for Yes {
        fn proceed_without_restore_point(&self, _r: &str) -> bool {
            true
        }
    }

    #[test]
    fn every_session_text_is_localized() {
        let c = Catalog::ukrainian().unwrap();
        let mut texts: Vec<Text> = Vec::new();
        for (high, low_mem, game_prio) in [
            (false, true, PriorityClass::Normal),
            (true, false, PriorityClass::Normal),
            (false, false, PriorityClass::High),
        ] {
            let (reg, power, store, procs) = (
                MemRegistry::new(),
                MemPower::new(guids::BALANCED),
                MemStore::new(),
                MemProcesses::new(),
            );
            let svc = MemServices::new(&[
                ("WSearch", ServiceState::Running),
                ("SysMain", ServiceState::Stopped),
            ]);
            let restore = FakeRestore(Ok(1));
            let mem = if low_mem {
                FakeMemory::new(8 << 30, 1 << 28)
            } else {
                FakeMemory::new(32 << 30, 20 << 30)
            };
            let game = procs.spawn(1, "game.exe", 1);
            procs.set_priority(&game, game_prio).unwrap();
            procs.spawn(2, "chrome.exe", 2);
            let mut engine = Engine::new(&reg, &power, &restore, &store)
                .unwrap()
                .with_processes(&procs)
                .with_services(&svc);
            let profile = GameProfile {
                exe: "game.exe".into(),
                game_priority: if high {
                    GamePriority::High
                } else {
                    GamePriority::AboveNormal
                },
                lower_background: vec![
                    "chrome.exe".into(),
                    "x.exe".into(),
                    "dwm.exe".into(),
                    "game.exe".into(),
                    "bad".into(),
                ],
                stop_services: vec!["WSearch".into(), "SysMain".into(), "wuauserv".into()],
                purge_standby_when_low: true,
            };
            let r = start_session(&mut engine, &procs, &mem, &profile, &game, &Yes).unwrap();
            texts.extend(r.applied);
            texts.extend(r.skipped);
        }
        texts.push(
            Text::new("gm.skip.error")
                .arg("what", "x")
                .arg("error", "y"),
        );
        let keys: std::collections::BTreeSet<&str> = texts.iter().map(|t| t.key.as_str()).collect();
        for t in &texts {
            assert!(c.contains(&t.key), "немає ключа {}", t.key);
            let r = c.render(t);
            assert!(!r.contains('{'), "незаповнений аргумент у {}: {r}", t.key);
        }
        for expected in [
            "gm.applied.game_above_normal",
            "gm.applied.game_high",
            "gm.applied.background",
            "gm.applied.service",
            "gm.applied.standby",
            "gm.skip.game_already_high",
            "gm.skip.not_running",
            "gm.skip.service_not_running",
            "gm.skip.memory_ok",
            "gm.warn.protected",
            "gm.warn.game_in_background",
            "gm.warn.bad_name",
            "gm.warn.service_not_allowed",
            "gm.warn.high_priority",
        ] {
            assert!(keys.contains(expected), "сценарії не покривають {expected}");
        }
        for p in [
            PriorityClass::Idle,
            PriorityClass::BelowNormal,
            PriorityClass::Normal,
            PriorityClass::AboveNormal,
            PriorityClass::High,
            PriorityClass::Realtime,
        ] {
            assert!(c.contains(p.text_key()));
        }
    }
}
