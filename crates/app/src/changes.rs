//! Куди йдуть зміни системи (етап 3б з `docs/SERVICE.md`): у службу OptimaX, якщо вона доступна,
//! інакше — у рушій у цьому процесі (з правами адміністратора).
//!
//! Журнал один (`%ProgramData%\OptimaX\<SID>\journal.json`), а писати його можуть двоє — служба
//! й програма без служби. Тому одночасно працює лише один шлях: перейшли на службу — свій рушій
//! забуваємо (наступного разу він перечитає журнал), служба сама перечитує журнал перед кожною
//! командою. Плани служби живуть у ній: програма знає лише їхні номери.

use crate::link::Link;
use crate::state::{MEMORY, POWER, PROCESSES, REGISTRY, RESTORE, SERVICES};
use optimax_backend::changes::{ApplyReply, PlanReply};
use optimax_service::client::Client;
use optimax_service::protocol::Command;
use optimax_service::ServiceError;
use optimax_tweaks::{
    ApplyOutcome, Changes, Confirm, Desired, Engine, FileStore, JournalEntry, Plan,
    RollbackSummary, TweakError,
};
use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};

/// Скільки планів служби пам'ятаємо (стільки ж тримає служба).
const PLANS_KEPT: usize = 32;

/// Журнал рушія в цьому процесі: створюється раз і живе до кінця роботи програми (рушій тримає
/// на нього посилання).
static STORE: OnceLock<FileStore> = OnceLock::new();

fn local_engine() -> Result<Engine<'static>, TweakError> {
    let store = match STORE.get() {
        Some(s) => s,
        None => {
            let s = FileStore::default_location()?;
            STORE.get_or_init(|| s)
        }
    };
    Ok(Engine::new(&REGISTRY, &POWER, &RESTORE, store)?
        .with_processes(&PROCESSES)
        .with_services(&SERVICES)
        .with_memory(&MEMORY))
}

fn remote_err(e: &ServiceError) -> TweakError {
    match e {
        // Служба відповіла відмовою — текст її рушія.
        ServiceError::Refused(m) => TweakError::Remote(m.clone()),
        other => TweakError::Remote(format!("служба OptimaX не відповіла: {other}")),
    }
}

/// Зміни системи: служба або рушій у процесі.
pub struct ChangeHub {
    link: Arc<Link>,
    local: Option<Engine<'static>>,
    remote: Option<Client>,
    /// Плани, складені службою: що показано користувачу → номер у службі.
    remote_plans: VecDeque<(Plan, u64)>,
    /// Номер цього запуску програми (служба створює нову точку відновлення для нової сесії).
    session: u64,
}

impl ChangeHub {
    /// Нічого не відкриває, доки не попросять.
    pub fn new(link: Arc<Link>) -> Self {
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self {
            link,
            local: None,
            remote: None,
            remote_plans: VecDeque::new(),
            session: (u64::from(std::process::id()) << 32) | (started & 0xFFFF_FFFF),
        }
    }

    /// Рушій у процесі (служба недоступна). Створюється заново після роботи через службу, щоб
    /// перечитати журнал.
    fn local(&mut self) -> Result<&mut Engine<'static>, TweakError> {
        self.remote = None;
        self.remote_plans.clear();
        if self.local.is_none() {
            self.local = Some(local_engine()?);
        }
        self.local
            .as_mut()
            .ok_or(TweakError::Unavailable("рушій змін"))
    }

    /// З'єднання зі службою, якщо вона доступна.
    fn remote(&mut self) -> Option<&mut Client> {
        if !self.link.service_usable() {
            self.remote = None;
            return None;
        }
        if self.remote.is_none() {
            match Client::connect() {
                Ok(c) => {
                    tracing::info!("зміни системи — через службу OptimaX");
                    self.local = None;
                    self.remote = Some(c);
                }
                Err(e) => {
                    self.link.forget_service(&e);
                    return None;
                }
            }
        }
        self.remote.as_mut()
    }

    /// Команда службі. `None` — служба недоступна (діяти самим). Обрив з'єднання — помилка, а
    /// наступна дія піде вже без служби.
    fn call<T: serde::de::DeserializeOwned>(
        &mut self,
        cmd: Command,
    ) -> Option<Result<T, TweakError>> {
        let c = self.remote()?;
        let r = c.json::<T>(cmd);
        if let Err(e) = &r {
            if !matches!(e, ServiceError::Refused(_)) {
                self.link.forget_service(e);
                self.remote = None;
                self.remote_plans.clear();
            }
        }
        Some(r.map_err(|e| remote_err(&e)))
    }

    fn remote_apply(
        &mut self,
        plan_id: u64,
        confirm: &dyn Confirm,
    ) -> Option<Result<ApplyOutcome, TweakError>> {
        let session = self.session;
        let send = |hub: &mut Self, allow| {
            hub.call::<ApplyReply>(Command::TweakApply {
                plan_id,
                allow_without_restore_point: allow,
                session,
            })
        };
        let reply = match send(self, false)? {
            Ok(ApplyReply::NeedsConsent { reason }) => {
                if !confirm.proceed_without_restore_point(&reason) {
                    return Some(Err(TweakError::Cancelled));
                }
                send(self, true)?
            }
            other => other,
        };
        Some(match reply {
            Ok(ApplyReply::Applied { outcome }) => {
                self.remote_plans.retain(|(_, id)| *id != plan_id);
                Ok(outcome)
            }
            Ok(ApplyReply::NeedsConsent { .. }) => Err(TweakError::Cancelled),
            Err(e) => Err(e),
        })
    }
}

impl Changes for ChangeHub {
    fn plan(&mut self, desired: &Desired) -> Result<Plan, TweakError> {
        let value =
            serde_json::to_value(desired).map_err(|e| TweakError::Journal(e.to_string()))?;
        match self.call::<PlanReply>(Command::TweakPlan { desired: value }) {
            Some(Ok(r)) => {
                if self.remote_plans.len() >= PLANS_KEPT {
                    self.remote_plans.pop_front();
                }
                self.remote_plans.push_back((r.plan.clone(), r.id));
                Ok(r.plan)
            }
            Some(Err(e @ TweakError::Remote(_))) if self.remote.is_none() => {
                // З'єднання обірвалось — план нічого не змінює, складемо його самі.
                tracing::warn!("{e}");
                self.local()?.plan(desired)
            }
            Some(Err(e)) => Err(e),
            None => self.local()?.plan(desired),
        }
    }

    fn apply(&mut self, plan: &Plan, confirm: &dyn Confirm) -> Result<ApplyOutcome, TweakError> {
        if plan.is_noop() {
            return Ok(ApplyOutcome {
                entry_ids: Vec::new(),
                restore_point: None,
            });
        }
        let known = self
            .remote_plans
            .iter()
            .rev()
            .find(|(p, _)| p == plan)
            .map(|(_, id)| *id);
        if let Some(id) = known {
            if let Some(r) = self.remote_apply(id, confirm) {
                return r;
            }
        }
        // План складено без служби (або служба зникла) — рушій у процесі перевірить, що
        // значення не змінилися з моменту перегляду.
        self.local()?.apply(plan, confirm)
    }

    fn purge_standby(&mut self) -> Result<u64, TweakError> {
        match self.call::<u64>(Command::PurgeStandby) {
            Some(r) => r,
            None => self.local()?.purge_standby(),
        }
    }

    fn entries(&mut self) -> Result<Vec<JournalEntry>, TweakError> {
        match self.call::<Vec<JournalEntry>>(Command::JournalEntries) {
            Some(Ok(e)) => Ok(e),
            Some(Err(e)) if self.remote.is_some() => Err(e),
            // Обрив — читаємо журнал самі.
            Some(Err(_)) | None => Ok(Engine::entries(self.local()?).to_vec()),
        }
    }

    fn rollback(&mut self, id: u64) -> Result<(), TweakError> {
        match self.call::<RollbackSummary>(Command::JournalRollback { ids: vec![id] }) {
            Some(Ok(s)) => match s.failed.into_iter().next() {
                Some((_, why)) => Err(TweakError::Remote(why)),
                None => Ok(()),
            },
            Some(Err(e)) => Err(e),
            None => self.local()?.rollback(id),
        }
    }

    fn rollback_ids(&mut self, ids: &[u64]) -> RollbackSummary {
        let cmd = Command::JournalRollback { ids: ids.to_vec() };
        match self.call::<RollbackSummary>(cmd) {
            Some(Ok(s)) => s,
            Some(Err(e)) => failed_all(ids, &e),
            None => match self.local() {
                Ok(engine) => engine.rollback_ids(ids),
                Err(e) => failed_all(ids, &e),
            },
        }
    }

    fn rollback_all(&mut self) -> RollbackSummary {
        match self.call::<RollbackSummary>(Command::JournalRollbackAll) {
            Some(Ok(s)) => s,
            Some(Err(e)) => failed_all(&[0], &e),
            None => match self.local() {
                Ok(engine) => engine.rollback_all(),
                Err(e) => failed_all(&[0], &e),
            },
        }
    }

    fn recover_pending(&mut self) -> Result<usize, TweakError> {
        match self.call::<usize>(Command::JournalRecover) {
            Some(r) => r,
            None => self.local()?.recover_pending(),
        }
    }
}

fn failed_all(ids: &[u64], e: &TweakError) -> RollbackSummary {
    RollbackSummary {
        rolled_back: Vec::new(),
        failed: ids.iter().map(|id| (*id, e.to_string())).collect(),
    }
}
