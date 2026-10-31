//! Застосування й відкат змін із журналом.

use crate::catalog::{Desired, GpuPref, TweakId};
use crate::change::{
    describe_power_mode, describe_registry, describe_scheme, describe_service, Plan, PlannedChange,
    SettingTarget, SettingValue,
};
use crate::journal::{EntryState, JournalEntry, JournalStore};
use crate::restore::{RestorePointApi, RestorePointStatus};
use crate::startup::{self, exe_from_command, is_plain_file_name, is_system_path, StartupSource};
use crate::TweakError;
use optimax_core::power::PowerApi;
use optimax_core::process::{is_protected_process, PriorityClass, ProcessApi};
use optimax_core::registry::{RegHive, RegValue, RegistryWriteApi};
use optimax_core::service::{is_stoppable_service, ServiceApi};
use optimax_core::{CoreError, Text};
use std::time::{SystemTime, UNIX_EPOCH};

/// Ключ ігрового режиму.
pub const GAME_MODE_KEY: &str = r"Software\Microsoft\GameBar";
/// Значення ігрового режиму (REG_DWORD: 1 — увімкнено, 0 — вимкнено, відсутнє — увімкнено).
pub const GAME_MODE_VALUE: &str = "AutoGameModeEnabled";
/// Ключ вибору GPU для exe.
const GPU_PREF_KEY: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";
/// Опис точки відновлення.
pub const RESTORE_POINT_DESCRIPTION: &str = "OptimaX: перед зміною налаштувань";

/// Питання до користувача, на які рушій не відповідає сам.
pub trait Confirm {
    /// Точку відновлення створити не вдалося — продовжувати лише з журналом OptimaX?
    fn proceed_without_restore_point(&self, reason: &str) -> bool;
}

/// Результат застосування.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyOutcome {
    /// Номери записів журналу.
    pub entry_ids: Vec<u64>,
    /// Точка відновлення, якщо її створено саме зараз (перша зміна в сесії).
    pub restore_point: Option<RestorePointStatus>,
}

/// Результат масового відкату.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RollbackSummary {
    /// Відкочені записи.
    pub rolled_back: Vec<u64>,
    /// Записи, які відкотити не вдалося, з причиною.
    pub failed: Vec<(u64, String)>,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Ціль, що зникає сама (процес завершився) — тоді відкочувати нічого.
fn is_ephemeral(target: &SettingTarget) -> bool {
    matches!(target, SettingTarget::ProcessPriority { .. })
}

/// Запис журналу веде саме туди, куди рушій пише для цього твіку, і з тим самим типом значення.
/// Журнал лежить у теці, доступній на запис без прав адміністратора, тому перед відкатом
/// (запис із правами адміністратора) підроблений запис не має вести до довільного ключа чи служби.
fn is_known_target(entry: &JournalEntry) -> bool {
    let reg_values = |ok: fn(&RegValue) -> bool| {
        [&entry.old, &entry.new].into_iter().all(|v| match v {
            SettingValue::Registry(v) => v.as_ref().is_none_or(ok),
            _ => false,
        })
    };
    match (TweakId::parse(&entry.tweak_id), &entry.target) {
        (Some(TweakId::GameMode), SettingTarget::Registry { hive, path, name }) => {
            *hive == RegHive::CurrentUser
                && path == GAME_MODE_KEY
                && name == GAME_MODE_VALUE
                && reg_values(|v| matches!(v, RegValue::Dword(_)))
        }
        (Some(TweakId::GpuPreference), SettingTarget::Registry { hive, path, name }) => {
            *hive == RegHive::CurrentUser
                && path == GPU_PREF_KEY
                && !name.is_empty()
                && reg_values(|v| matches!(v, RegValue::String(_)))
        }
        (Some(TweakId::StartupApp), SettingTarget::Registry { hive, path, name }) => {
            StartupSource::ALL
                .iter()
                .any(|s| s.hive() == *hive && s.approved_key() == *path)
                && !name.is_empty()
                && reg_values(|v| matches!(v, RegValue::Binary(_)))
        }
        (Some(TweakId::PowerScheme), SettingTarget::PowerScheme)
        | (Some(TweakId::StandbyPurge), SettingTarget::StandbyList)
        | (Some(TweakId::PowerMode), SettingTarget::PowerMode) => true,
        (Some(TweakId::ProcessPriority), SettingTarget::ProcessPriority { process }) => {
            !is_protected_process(&process.name)
        }
        (Some(TweakId::Service), SettingTarget::Service { name }) => is_stoppable_service(name),
        _ => false,
    }
}

/// Рушій змін. Одна «сесія» = один екземпляр: точка відновлення створюється перед першою зміною.
pub struct Engine<'a> {
    registry: &'a dyn RegistryWriteApi,
    power: &'a dyn PowerApi,
    restore: &'a dyn RestorePointApi,
    store: &'a dyn JournalStore,
    processes: Option<&'a dyn ProcessApi>,
    services: Option<&'a dyn ServiceApi>,
    entries: Vec<JournalEntry>,
    session_restore: Option<RestorePointStatus>,
}

impl<'a> Engine<'a> {
    /// Створює рушій і завантажує журнал.
    pub fn new(
        registry: &'a dyn RegistryWriteApi,
        power: &'a dyn PowerApi,
        restore: &'a dyn RestorePointApi,
        store: &'a dyn JournalStore,
    ) -> Result<Self, TweakError> {
        Ok(Self {
            registry,
            power,
            restore,
            store,
            processes: None,
            services: None,
            entries: store.load()?,
            session_restore: None,
        })
    }

    /// Дозволяє змінювати пріоритети процесів.
    #[must_use]
    pub fn with_processes(mut self, processes: &'a dyn ProcessApi) -> Self {
        self.processes = Some(processes);
        self
    }

    /// Дозволяє тимчасово зупиняти служби зі списку дозволених.
    #[must_use]
    pub fn with_services(mut self, services: &'a dyn ServiceApi) -> Self {
        self.services = Some(services);
        self
    }

    fn processes(&self) -> Result<&'a dyn ProcessApi, TweakError> {
        self.processes
            .ok_or(TweakError::Unavailable("керування процесами"))
    }

    fn services(&self) -> Result<&'a dyn ServiceApi, TweakError> {
        self.services
            .ok_or(TweakError::Unavailable("керування службами"))
    }

    /// Усі записи журналу.
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// Записи, що зараз діють (застосовані й не відкочені).
    pub fn active(&self) -> impl Iterator<Item = &JournalEntry> {
        self.entries
            .iter()
            .filter(|e| e.state == EntryState::Applied)
    }

    fn save(&self) -> Result<(), TweakError> {
        self.store.save(&self.entries)
    }

    fn read(&self, target: &SettingTarget) -> Result<SettingValue, TweakError> {
        Ok(match target {
            SettingTarget::Registry { hive, path, name } => {
                SettingValue::Registry(self.registry.read_value(*hive, path, name)?)
            }
            SettingTarget::PowerScheme => SettingValue::PowerScheme(self.power.active_scheme()?),
            SettingTarget::ProcessPriority { process } => {
                SettingValue::Priority(self.processes()?.priority(process)?)
            }
            SettingTarget::Service { name } => SettingValue::Service(self.services()?.state(name)?),
            SettingTarget::StandbyList => SettingValue::Action,
            SettingTarget::PowerMode => SettingValue::PowerMode(self.power.overlay_scheme()?),
        })
    }

    fn write(&self, target: &SettingTarget, value: &SettingValue) -> Result<(), TweakError> {
        match (target, value) {
            (SettingTarget::Registry { hive, path, name }, SettingValue::Registry(Some(v))) => {
                Ok(self.registry.write_value(*hive, path, name, v)?)
            }
            (SettingTarget::Registry { hive, path, name }, SettingValue::Registry(None)) => {
                Ok(self.registry.delete_value(*hive, path, name)?)
            }
            (SettingTarget::PowerScheme, SettingValue::PowerScheme(guid)) => {
                Ok(self.power.set_active_scheme(guid)?)
            }
            (SettingTarget::ProcessPriority { process }, SettingValue::Priority(p)) => {
                Ok(self.processes()?.set_priority(process, *p)?)
            }
            (SettingTarget::Service { name }, SettingValue::Service(state)) => {
                let services = self.services()?;
                match state {
                    optimax_core::service::ServiceState::Running => Ok(services.start(name)?),
                    optimax_core::service::ServiceState::Stopped => Ok(services.stop(name)?),
                    optimax_core::service::ServiceState::Other(_) => Err(TweakError::Forbidden(
                        format!("{name}: перехідний стан не встановлюється"),
                    )),
                }
            }
            (SettingTarget::PowerMode, SettingValue::PowerMode(guid)) => {
                Ok(self.power.set_overlay_scheme(guid.as_deref())?)
            }
            _ => Err(TweakError::Journal(format!(
                "{}: тип значення не відповідає місцю зберігання",
                target.location()
            ))),
        }
    }

    /// Пише значення й перевіряє, що система справді його прийняла.
    fn write_verified(
        &self,
        target: &SettingTarget,
        value: &SettingValue,
    ) -> Result<(), TweakError> {
        self.write(target, value)?;
        // PowerMode використовує недокументований API; не перевіряємо повернене значення,
        // бо PowerGetEffectiveOverlayScheme може повертати ефективний, а не встановлений режим.
        if matches!(target, SettingTarget::PowerMode) {
            return Ok(());
        }
        if &self.read(target)? == value {
            Ok(())
        } else {
            Err(TweakError::VerifyFailed {
                location: target.location(),
            })
        }
    }

    /// Будує план: читає поточний стан і перевіряє тип. Нічого не змінює.
    pub fn plan(&self, desired: &Desired) -> Result<Plan, TweakError> {
        let info = desired.tweak().info();
        let tweak_id = desired.tweak().as_str().to_string();
        let changes = match desired {
            Desired::GameMode(on) => {
                let target = SettingTarget::Registry {
                    hive: RegHive::CurrentUser,
                    path: GAME_MODE_KEY.into(),
                    name: GAME_MODE_VALUE.into(),
                };
                let current = self.registry.read_value(
                    RegHive::CurrentUser,
                    GAME_MODE_KEY,
                    GAME_MODE_VALUE,
                )?;
                // Читаємо → перевіряємо тип → лише тоді плануємо запис.
                let enabled_now = match &current {
                    None => true,
                    Some(RegValue::Dword(v)) => *v != 0,
                    Some(other) => {
                        return Err(TweakError::TypeMismatch {
                            location: target.location(),
                            expected: "REG_DWORD",
                            found: other.type_name(),
                        })
                    }
                };
                if enabled_now == *on {
                    Vec::new()
                } else {
                    let new = RegValue::Dword(u32::from(*on));
                    vec![PlannedChange {
                        tweak_id,
                        old_display: describe_registry(current.as_ref()),
                        new_display: describe_registry(Some(&new)),
                        target,
                        old: SettingValue::Registry(current),
                        new: SettingValue::Registry(Some(new)),
                    }]
                }
            }
            Desired::PowerScheme(guid) => {
                let guid = guid.trim().to_ascii_lowercase();
                let schemes = self.power.schemes()?;
                let Some(target_scheme) = schemes.iter().find(|s| s.guid == guid) else {
                    return Err(TweakError::UnknownScheme(guid));
                };
                let current = self.power.active_scheme()?;
                if current == guid {
                    Vec::new()
                } else {
                    let current_name = schemes
                        .iter()
                        .find(|s| s.guid == current)
                        .and_then(|s| s.name.clone());
                    vec![PlannedChange {
                        tweak_id,
                        target: SettingTarget::PowerScheme,
                        old_display: describe_scheme(&current, current_name.as_deref()),
                        new_display: describe_scheme(&guid, target_scheme.name.as_deref()),
                        old: SettingValue::PowerScheme(current),
                        new: SettingValue::PowerScheme(guid),
                    }]
                }
            }
            Desired::ProcessPriority { process, priority } => {
                // Захищені процеси й Realtime — ніколи, хоч би що просили.
                if is_protected_process(&process.name) {
                    return Err(TweakError::Forbidden(format!(
                        "{}: захищений процес",
                        process.name
                    )));
                }
                if *priority == PriorityClass::Realtime {
                    return Err(TweakError::Forbidden(
                        "пріоритет Realtime може заблокувати систему".into(),
                    ));
                }
                let current = self.processes()?.priority(process)?;
                if current == *priority {
                    Vec::new()
                } else {
                    vec![PlannedChange {
                        tweak_id,
                        target: SettingTarget::ProcessPriority {
                            process: process.clone(),
                        },
                        old_display: Text::new(current.text_key()),
                        new_display: Text::new(priority.text_key()),
                        old: SettingValue::Priority(current),
                        new: SettingValue::Priority(*priority),
                    }]
                }
            }
            Desired::Service { name, state } => {
                if !is_stoppable_service(name) {
                    return Err(TweakError::Forbidden(format!(
                        "служба {name} не входить до списку дозволених"
                    )));
                }
                let current = self.services()?.state(name)?;
                if current == *state {
                    Vec::new()
                } else {
                    vec![PlannedChange {
                        tweak_id,
                        target: SettingTarget::Service { name: name.clone() },
                        old_display: describe_service(current),
                        new_display: describe_service(*state),
                        old: SettingValue::Service(current),
                        new: SettingValue::Service(*state),
                    }]
                }
            }
            Desired::PowerMode(guid) => {
                if !self.power.overlay_supported() {
                    return Err(TweakError::Forbidden(
                        "overlay режим живлення не підтримується на цій конфігурації".into(),
                    ));
                }
                let current = self.power.overlay_scheme()?;
                let normalized = guid.as_deref().map(|g| g.trim().to_ascii_lowercase());
                if current == normalized {
                    Vec::new()
                } else {
                    vec![PlannedChange {
                        tweak_id,
                        target: SettingTarget::PowerMode,
                        old_display: describe_power_mode(current.as_deref()),
                        new_display: describe_power_mode(normalized.as_deref()),
                        old: SettingValue::PowerMode(current),
                        new: SettingValue::PowerMode(normalized),
                    }]
                }
            }
            Desired::GpuPreference {
                exe_path,
                preference,
            } => {
                let target = SettingTarget::Registry {
                    hive: RegHive::CurrentUser,
                    path: GPU_PREF_KEY.into(),
                    name: exe_path.clone(),
                };
                let current =
                    self.registry
                        .read_value(RegHive::CurrentUser, GPU_PREF_KEY, exe_path)?;
                let new_value = match preference {
                    GpuPref::Default => None,
                    GpuPref::PowerSaving => Some(optimax_core::registry::RegValue::String(
                        "GpuPreference=1;".into(),
                    )),
                    GpuPref::HighPerformance => Some(optimax_core::registry::RegValue::String(
                        "GpuPreference=2;".into(),
                    )),
                };
                if current == new_value {
                    Vec::new()
                } else {
                    vec![PlannedChange {
                        tweak_id,
                        old_display: describe_registry(current.as_ref()),
                        new_display: describe_registry(new_value.as_ref()),
                        target,
                        old: SettingValue::Registry(current),
                        new: SettingValue::Registry(new_value),
                    }]
                }
            }
            Desired::StartupApp {
                source,
                name,
                enabled,
            } => self.plan_startup(*source, name, *enabled, tweak_id)?,
        };
        Ok(Plan { info, changes })
    }

    /// Автозапуск: лише прапорець у StartupApproved; запис у Run не змінюється.
    fn plan_startup(
        &self,
        source: StartupSource,
        name: &str,
        enabled: bool,
        tweak_id: String,
    ) -> Result<Vec<PlannedChange>, TweakError> {
        let hive = source.hive();
        match source.run_key() {
            Some(run) => {
                // Запис має існувати, а системні (у теці Windows) не вимикаємо.
                let command = match self.registry.read_value(hive, run, name)? {
                    Some(RegValue::String(s) | RegValue::ExpandString(s)) => s,
                    Some(other) => {
                        return Err(TweakError::TypeMismatch {
                            location: format!("{run}\\{name}"),
                            expected: "REG_SZ",
                            found: other.type_name(),
                        })
                    }
                    None => {
                        return Err(TweakError::Forbidden(format!(
                            "{name}: такого запису автозапуску немає"
                        )))
                    }
                };
                let system_root = optimax_core::sysdirs::windows_dir()
                    .map(|w| w.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if is_system_path(&exe_from_command(&command), &system_root) {
                    return Err(TweakError::Forbidden(format!(
                        "{name}: системний компонент Windows — не вимикаємо"
                    )));
                }
            }
            None if !is_plain_file_name(name) => {
                return Err(TweakError::Forbidden(format!(
                    "{name}: очікувалось ім'я файлу з теки автозавантаження"
                )))
            }
            None => {}
        }
        let key = source.approved_key();
        let target = SettingTarget::Registry {
            hive,
            path: key.clone(),
            name: name.to_string(),
        };
        let current = self.registry.read_value(hive, &key, name)?;
        let enabled_now =
            startup::is_enabled(current.as_ref()).map_err(|found| TweakError::TypeMismatch {
                location: target.location(),
                expected: "REG_BINARY",
                found,
            })?;
        if enabled_now == enabled {
            return Ok(Vec::new());
        }
        let state = |on: bool| {
            Text::new(if on {
                "tweak.startup_app.enabled"
            } else {
                "tweak.startup_app.disabled"
            })
        };
        let new = startup::approved_value(enabled, startup::filetime_now());
        Ok(vec![PlannedChange {
            tweak_id,
            old_display: state(enabled_now),
            new_display: state(enabled),
            target,
            old: SettingValue::Registry(current),
            new: SettingValue::Registry(Some(new)),
        }])
    }

    fn ensure_restore_point(
        &mut self,
        confirm: &dyn Confirm,
    ) -> Result<Option<RestorePointStatus>, TweakError> {
        if self.session_restore.is_some() {
            return Ok(None);
        }
        let status = match self.restore.create(RESTORE_POINT_DESCRIPTION) {
            Ok(sequence) => {
                tracing::info!("створено точку відновлення №{sequence}");
                RestorePointStatus::Created { sequence }
            }
            Err(e) => {
                let reason = e.to_string();
                tracing::warn!("точку відновлення не створено: {reason}");
                if !confirm.proceed_without_restore_point(&reason) {
                    return Err(TweakError::Cancelled);
                }
                RestorePointStatus::SkippedByUser { reason }
            }
        };
        self.session_restore = Some(status.clone());
        Ok(Some(status))
    }

    /// Застосовує план. Перед першою зміною в сесії — точка відновлення.
    /// Будь-яка помилка → вже зроблені зміни цього плану відкочуються.
    pub fn apply(
        &mut self,
        plan: &Plan,
        confirm: &dyn Confirm,
    ) -> Result<ApplyOutcome, TweakError> {
        if plan.is_noop() {
            return Ok(ApplyOutcome {
                entry_ids: Vec::new(),
                restore_point: None,
            });
        }
        let restore_point = self.ensure_restore_point(confirm)?;
        let mut done: Vec<u64> = Vec::new();
        for change in &plan.changes {
            if let Err(e) = self.apply_one(change) {
                for id in done.iter().rev() {
                    if let Err(re) = self.rollback(*id) {
                        tracing::error!("автовідкат запису #{id} не вдався: {re}");
                    }
                }
                return Err(e);
            }
            if let Some(last) = self.entries.last() {
                done.push(last.id);
            }
        }
        Ok(ApplyOutcome {
            entry_ids: done,
            restore_point,
        })
    }

    fn apply_one(&mut self, change: &PlannedChange) -> Result<(), TweakError> {
        // Значення могло змінитися після того, як користувач переглянув план.
        if self.read(&change.target)? != change.old {
            return Err(TweakError::Stale {
                location: change.target.location(),
            });
        }
        let id = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        // Спершу журнал, потім зміна: якщо програма впаде, recover_pending доведе справу до кінця.
        self.entries.push(JournalEntry {
            id,
            tweak_id: change.tweak_id.clone(),
            target: change.target.clone(),
            old: change.old.clone(),
            new: change.new.clone(),
            created_unix: now_unix(),
            state: EntryState::Pending,
            restore_point: self.session_restore.clone(),
        });
        self.save()?;
        tracing::info!(
            "зміна #{id}: {} — {:?} → {:?}",
            change.target.location(),
            change.old,
            change.new
        );

        let result = self.write_verified(&change.target, &change.new);
        let state = match &result {
            Ok(()) => EntryState::Applied,
            Err(e) => {
                // Повертаємо старе значення цієї зміни, якщо запис пройшов частково.
                if let Err(re) = self.write(&change.target, &change.old) {
                    tracing::error!("не вдалося повернути {}: {re}", change.target.location());
                }
                EntryState::Failed {
                    error: e.to_string(),
                }
            }
        };
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.state = state;
        }
        self.save()?;
        result
    }

    /// Виконує разову дію без відкату (очищення standby) і записує її в журнал.
    pub fn perform(
        &mut self,
        tweak: crate::TweakId,
        target: SettingTarget,
        action: impl FnOnce() -> Result<(), CoreError>,
    ) -> Result<u64, TweakError> {
        let id = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        self.entries.push(JournalEntry {
            id,
            tweak_id: tweak.as_str().into(),
            target: target.clone(),
            old: SettingValue::Action,
            new: SettingValue::Action,
            created_unix: now_unix(),
            state: EntryState::Pending,
            restore_point: self.session_restore.clone(),
        });
        self.save()?;
        let result = action();
        let state = match &result {
            Ok(()) => EntryState::Performed,
            Err(e) => EntryState::Failed {
                error: e.to_string(),
            },
        };
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.state = state;
        }
        tracing::info!("дія #{id}: {} — {result:?}", target.location());
        self.save()?;
        result.map(|()| id).map_err(TweakError::from)
    }

    /// Відкочує один запис. Якщо значення після нас хтось змінив — відмова (не затираємо чуже).
    pub fn rollback(&mut self, id: u64) -> Result<(), TweakError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.id == id && e.state == EntryState::Applied)
            .cloned()
            .ok_or(TweakError::NoSuchEntry(id))?;
        if !is_known_target(&entry) {
            return Err(TweakError::Forbidden(format!(
                "запис #{id} ({}) не відповідає жодному твіку OptimaX — не відкочуємо",
                entry.target.location()
            )));
        }
        match self.read(&entry.target) {
            Ok(current) if current != entry.new => {
                return Err(TweakError::Conflict {
                    location: entry.target.location(),
                })
            }
            Ok(_) => self.write_verified(&entry.target, &entry.old)?,
            // Процес уже завершився — його пріоритет відновлювати нікому.
            Err(TweakError::Core(CoreError::NotFound(_))) if is_ephemeral(&entry.target) => {}
            Err(e) => return Err(e),
        }
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.state = EntryState::RolledBack {
                at_unix: now_unix(),
            };
        }
        tracing::info!("відкочено #{id}: {}", entry.target.location());
        self.save()
    }

    /// Відкочує всі діючі зміни, від найновішої до найстарішої. Помилки не зупиняють решту.
    pub fn rollback_all(&mut self) -> RollbackSummary {
        let ids: Vec<u64> = self.active().map(|e| e.id).collect();
        self.rollback_ids(&ids)
    }

    /// Відкочує задані записи (напр. лише зміни однієї ігрової сесії), від найновішого.
    pub fn rollback_ids(&mut self, ids: &[u64]) -> RollbackSummary {
        let mut ids = ids.to_vec();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.dedup();
        let mut summary = RollbackSummary::default();
        for id in ids {
            match self.rollback(id) {
                Ok(()) => summary.rolled_back.push(id),
                Err(e) => summary.failed.push((id, e.to_string())),
            }
        }
        summary
    }

    /// Доводить до кінця записи, що лишилися `Pending` після аварійного завершення.
    /// Повертає кількість оброблених записів.
    pub fn recover_pending(&mut self) -> Result<usize, TweakError> {
        let pending: Vec<JournalEntry> = self
            .entries
            .iter()
            .filter(|e| e.state == EntryState::Pending)
            .cloned()
            .collect();
        for entry in &pending {
            if !is_known_target(entry) {
                tracing::warn!(
                    "запис журналу #{} ({}) не відповідає жодному твіку OptimaX — пропущено",
                    entry.id,
                    entry.target.location()
                );
                if let Some(e) = self.entries.iter_mut().find(|e| e.id == entry.id) {
                    e.state = EntryState::Failed {
                        error: "запис не відповідає жодному твіку OptimaX — не виконано".into(),
                    };
                }
                continue;
            }
            let current = match self.read(&entry.target) {
                Ok(v) => v,
                Err(TweakError::Core(CoreError::NotFound(_))) if is_ephemeral(&entry.target) => {
                    if let Some(e) = self.entries.iter_mut().find(|e| e.id == entry.id) {
                        e.state = EntryState::RolledBack {
                            at_unix: now_unix(),
                        };
                    }
                    continue;
                }
                Err(e) => return Err(e),
            };
            let state = if entry.target == SettingTarget::StandbyList {
                EntryState::Failed {
                    error: "дію перервано збоєм; відкат не потрібен".into(),
                }
            } else if current == entry.new {
                match self.write_verified(&entry.target, &entry.old) {
                    Ok(()) => EntryState::RolledBack {
                        at_unix: now_unix(),
                    },
                    Err(e) => EntryState::Failed {
                        error: format!("автовідкат після збою не вдався: {e}"),
                    },
                }
            } else if current == entry.old {
                EntryState::Failed {
                    error: "зміну не було застосовано (збій до запису)".into(),
                }
            } else {
                EntryState::Failed {
                    error: "стан невідомий: значення змінене поза OptimaX".into(),
                }
            };
            tracing::warn!("відновлення після збою, запис #{}: {state:?}", entry.id);
            if let Some(e) = self.entries.iter_mut().find(|e| e.id == entry.id) {
                e.state = state;
            }
        }
        if !pending.is_empty() {
            self.save()?;
        }
        Ok(pending.len())
    }
}

#[cfg(test)]
mod tests;
