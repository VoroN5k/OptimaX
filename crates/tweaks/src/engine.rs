//! Застосування й відкат змін із журналом.

use crate::catalog::Desired;
use crate::change::{
    describe_registry, describe_scheme, Plan, PlannedChange, SettingTarget, SettingValue,
};
use crate::journal::{EntryState, JournalEntry, JournalStore};
use crate::restore::{RestorePointApi, RestorePointStatus};
use crate::TweakError;
use optimax_core::power::PowerApi;
use optimax_core::registry::{RegHive, RegValue, RegistryWriteApi};
use std::time::{SystemTime, UNIX_EPOCH};

/// Ключ ігрового режиму.
pub const GAME_MODE_KEY: &str = r"Software\Microsoft\GameBar";
/// Значення ігрового режиму (REG_DWORD: 1 — увімкнено, 0 — вимкнено, відсутнє — увімкнено).
pub const GAME_MODE_VALUE: &str = "AutoGameModeEnabled";
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

/// Рушій змін. Одна «сесія» = один екземпляр: точка відновлення створюється перед першою зміною.
pub struct Engine<'a> {
    registry: &'a dyn RegistryWriteApi,
    power: &'a dyn PowerApi,
    restore: &'a dyn RestorePointApi,
    store: &'a dyn JournalStore,
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
            entries: store.load()?,
            session_restore: None,
        })
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
        };
        Ok(Plan { info, changes })
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

    /// Відкочує один запис. Якщо значення після нас хтось змінив — відмова (не затираємо чуже).
    pub fn rollback(&mut self, id: u64) -> Result<(), TweakError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.id == id && e.state == EntryState::Applied)
            .cloned()
            .ok_or(TweakError::NoSuchEntry(id))?;
        if self.read(&entry.target)? != entry.new {
            return Err(TweakError::Conflict {
                location: entry.target.location(),
            });
        }
        self.write_verified(&entry.target, &entry.old)?;
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
        let mut ids: Vec<u64> = self.active().map(|e| e.id).collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
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
            let current = self.read(&entry.target)?;
            let state = if current == entry.new {
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
