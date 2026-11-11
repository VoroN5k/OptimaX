//! Очищення диска без Tauri (етап 3в з `docs/SERVICE.md`): скан, сухий прогін, очищення, карантин,
//! журнал, «Що виросло», DISM і гібернація. Виконує служба (від імені того, хто під'єднався), а
//! без служби — сама програма; логіка одна ([`Cleaner`]).
//!
//! Клієнт не передає шляхів для видалення: лише корені об'єктів **з останнього скану того, хто
//! виконує** (служба звіряє їх зі своїм звітом), далі — номер плану сухого прогону. Правила,
//! чорний список і канонізацію шляхів перевіряє той самий рушій `optimax_cleanup`, що й без
//! служби (CLAUDE.md 11.3). Теки правил (`%LOCALAPPDATA%`, профіль) — користувача клієнта, з
//! токена його процесу ([`optimax_cleanup::Env::for_process`]).

use optimax_cleanup::journal::{CleanupJournal, JournalRecord, RunKind};
use optimax_cleanup::plan::{Action, ServiceNote, Skipped};
use optimax_cleanup::quarantine::{BatchState, Quarantine, QUARANTINE_DIR_NAME};
use optimax_cleanup::rules::{RuleSet, Verdict};
use optimax_cleanup::scan::{ScanOptions, ScanProgress, ScanReport};
use optimax_cleanup::snapshot::{growth, pick_baseline, Growth, Snapshot, SnapshotStore};
use optimax_cleanup::system::SystemTools;
use optimax_cleanup::{
    build_plan, execute, path, scan, CancelToken, CleanPlan, CleanupError, Env, ExecContext,
    ExecProgress, FsApi,
};
use optimax_core::service::ServiceApi;
use optimax_service::protocol::{Command, Reply};
use optimax_tweaks::restore::{RestorePointApi, RestorePointStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Опис точки відновлення для очищення.
const RESTORE_DESCRIPTION: &str = "OptimaX: очищення диска";
/// «Що виросло» — порівняння зі сканом приблизно тижневої давності.
const GROWTH_WINDOW_SECS: i64 = 7 * 86_400;
/// Скільки файлів показувати в сухому прогоні для кожного об'єкта.
const DRY_RUN_SAMPLE: usize = 200;
/// Скільки пропущених показувати.
const SKIPPED_SAMPLE: usize = 200;

/// Помилка очищення (переживає канал служби: UI розрізняє «потрібна згода без точки
/// відновлення», «заборонено» й «не знайдено»).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum CleanError {
    /// Точку відновлення не створено — потрібне рішення користувача.
    RestorePoint(String),
    /// Заборонено правилами безпеки.
    Forbidden(String),
    /// Не знайдено (план, скан, партія).
    NotFound(String),
    /// Інша помилка.
    Failed(String),
}

impl std::fmt::Display for CleanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RestorePoint(m) | Self::Forbidden(m) | Self::NotFound(m) | Self::Failed(m) => {
                f.write_str(m)
            }
        }
    }
}

impl From<CleanupError> for CleanError {
    fn from(e: CleanupError) -> Self {
        match e {
            CleanupError::NotDeletable(_) => Self::Forbidden(e.to_string()),
            CleanupError::NotFound(_) => Self::NotFound(e.to_string()),
            other => Self::Failed(other.to_string()),
        }
    }
}

/// Результат дії очищення.
pub type CleanResult<T> = Result<T, CleanError>;

fn failed(e: impl std::fmt::Display) -> CleanError {
    CleanError::Failed(e.to_string())
}

/// Налаштування карантину (з налаштувань програми).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuarantineSpec {
    /// Диск (`D:\`, файли в `D:\OptimaX\<SID>\Quarantine`); `None` — тека даних.
    pub drive: Option<String>,
    /// Термін, днів.
    pub days: u32,
}

/// Звіт скану з проблемами бази правил (правила, понижені до ⚪).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastScan {
    /// Звіт.
    pub report: ScanReport,
    /// Проблеми бази правил.
    pub rule_issues: Vec<String>,
}

/// Завершений скан.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanDone {
    /// Звіт.
    pub scan: LastScan,
    /// Звіт став звітом екрана «Очищення».
    pub kept: bool,
}

/// Об'єкт сухого прогону.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DryRunItem {
    /// Корінь правила.
    pub path: String,
    /// Правило.
    pub rule_id: String,
    /// Вердикт.
    pub verdict: Verdict,
    /// Дія.
    pub action: Action,
    /// Файлів.
    pub files: u64,
    /// Байтів.
    pub bytes: u64,
    /// Перші файли.
    pub sample: Vec<String>,
}

/// Сухий прогін.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DryRunView {
    /// Номер плану (виконати можна лише його).
    pub plan_id: u64,
    /// Об'єкти.
    pub items: Vec<DryRunItem>,
    /// До видалення (🟢).
    pub delete_bytes: u64,
    /// До карантину (🟡).
    pub quarantine_bytes: u64,
    /// Файлів.
    pub files: u64,
    /// Відкинуто ще на етапі плану (перші).
    pub skipped: Vec<Skipped>,
    /// Усього відкинуто.
    pub skipped_total: u64,
    /// Служби, які буде тимчасово зупинено.
    pub services: Vec<String>,
    /// Тека карантину.
    pub quarantine_dir: String,
    /// Карантин на тому ж диску, що й файли 🟡: місце звільниться лише після терміну.
    pub quarantine_same_volume: bool,
    /// Термін карантину, днів.
    pub quarantine_days: u32,
    /// Перед очищенням буде створено точку відновлення.
    pub restore_point_pending: bool,
}

/// Підсумок очищення.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExecView {
    /// Запис журналу.
    pub record: JournalRecord,
    /// Служби.
    pub services: Vec<ServiceNote>,
    /// Карантин на тому ж диску (місце ще не звільнено).
    pub quarantine_same_volume: bool,
    /// Скасовано посередині.
    pub cancelled: bool,
}

/// Партія карантину (без списку файлів).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BatchView {
    /// Ідентифікатор.
    pub id: String,
    /// Створено.
    pub created_unix: i64,
    /// Кінець карантину.
    pub expires_unix: i64,
    /// Байтів.
    pub bytes: u64,
    /// Файлів.
    pub files: u64,
    /// На тому ж диску, що й оригінали.
    pub same_volume: bool,
    /// Термін минув.
    pub expired: bool,
    /// Тека.
    pub dir: String,
}

/// Карантин: тека й активні партії.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantineState {
    /// Поточна тека карантину.
    pub dir: String,
    /// Карантин на системному диску.
    pub on_system_drive: bool,
    /// Активні партії.
    pub batches: Vec<BatchView>,
}

/// Рядок «Що виросло».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GrowthItem {
    /// Зміна.
    pub growth: Growth,
    /// Вердикт для шляху.
    pub verdict: Verdict,
    /// Правило.
    pub rule_id: Option<String>,
}

/// Що виросло.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GrowthView {
    /// Базовий знімок.
    pub since_unix: i64,
    /// Поточний знімок.
    pub now_unix: i64,
    /// Обидва знімки повні.
    pub full: bool,
    /// Зміни.
    pub items: Vec<GrowthItem>,
}

/// Хід і результати з минулого опитування служби ([`Command::CleanupPoll`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CleanupEvents {
    /// Останній хід скану.
    pub scan_progress: Option<ScanProgress>,
    /// Останній проміжний звіт (лише для показу).
    pub scan_partial: Option<LastScan>,
    /// Скан завершено.
    pub scan_done: Option<ScanDone>,
    /// Останній хід очищення.
    pub exec_progress: Option<ExecProgress>,
    /// Очищення завершено.
    pub exec_done: Option<CleanResult<ExecView>>,
}

/// Чий скан.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    /// Екран «Очищення»: звіт стає звітом екрана, попередній план забувається.
    Screen,
    /// Головний екран: звіт стає звітом екрана, лише якщо там ще порожньо.
    Background,
}

/// Система, на якій працює [`Cleaner`]: файли, служби, точка відновлення, штатні засоби й теки
/// користувача (служба — того, хто під'єднався; програма — свої).
pub trait CleanupPlatform: Send + Sync {
    /// Змінні правил (теки системи й користувача).
    fn env(&self) -> Env;
    /// Захищена тека даних користувача (нормалізована).
    fn data_dir(&self) -> CleanResult<String>;
    /// Захищена тека користувача на іншому диску (`D:\OptimaX\<SID>`), створюється з тим самим
    /// дескриптором безпеки.
    fn dir_on_drive(&self, drive: &str) -> CleanResult<String>;
    /// Системний диск (`C:\`).
    fn system_drive(&self) -> String;
    /// Вільно на диску, байтів.
    fn free_bytes(&self, drive: &str) -> Option<u64>;
    /// Поточний час (Unix-секунди).
    fn now_unix(&self) -> i64;
    /// Файлова система.
    fn fs(&self) -> &dyn FsApi;
    /// Служби Windows.
    fn services(&self) -> &dyn ServiceApi;
    /// Точка відновлення.
    fn restore(&self) -> &dyn RestorePointApi;
    /// DISM і powercfg.
    fn tools(&self) -> &dyn SystemTools;
    /// Хто зараз просить (PID клієнта служби): теки правил — його користувача.
    fn set_client(&self, _pid: u32) {}
}

#[derive(Default)]
struct State {
    rules: Option<Arc<RuleSet>>,
    rule_issues: Vec<String>,
    scan: Option<CancelToken>,
    exec: Option<CancelToken>,
    tool_busy: bool,
    report: Option<ScanReport>,
    plan: Option<(u64, Arc<CleanPlan>)>,
    next_plan: u64,
    restore: Option<RestorePointStatus>,
    session: Option<u64>,
}

impl State {
    fn busy(&self) -> Option<&'static str> {
        if self.scan.is_some() {
            Some("скан ще триває")
        } else if self.exec.is_some() {
            Some("очищення ще триває")
        } else if self.tool_busy {
            Some("штатне очищення Windows ще триває")
        } else {
            None
        }
    }
}

/// Очищення одного користувача: база правил, останній скан, план, точка відновлення сесії.
pub struct Cleaner {
    p: Box<dyn CleanupPlatform>,
    st: Mutex<State>,
}

/// Скан, що вже зайняв модуль (запускається в окремому потоці через [`ScanJob::run`]).
pub struct ScanJob {
    root: String,
    full: bool,
    keep: bool,
    cancel: CancelToken,
    rules: Arc<RuleSet>,
}

/// Очищення, що вже зайняло модуль і почало запис у журналі ([`ExecJob::run`]).
pub struct ExecJob {
    jid: u64,
    plan: Arc<CleanPlan>,
    rules: Arc<RuleSet>,
    quarantine: QuarantineSpec,
    cancel: CancelToken,
}

/// Знімає «зайнято» зі штатних засобів і карантину при виході (навіть після помилки).
struct ToolGuard<'a>(&'a Cleaner);

impl Drop for ToolGuard<'_> {
    fn drop(&mut self) {
        self.0.state().tool_busy = false;
    }
}

impl Cleaner {
    /// Над заданою системою. Нічого не читає, доки не попросять.
    pub fn new(platform: Box<dyn CleanupPlatform>) -> Self {
        Self {
            p: platform,
            st: Mutex::new(State::default()),
        }
    }

    /// Хто зараз просить (служба: PID клієнта).
    pub fn set_client(&self, pid: u32) {
        self.p.set_client(pid);
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// База правил: `fresh` — зібрати заново (теки користувача могли змінитися).
    fn rules_in(&self, st: &mut State, fresh: bool) -> Arc<RuleSet> {
        if let (false, Some(r)) = (fresh, &st.rules) {
            return r.clone();
        }
        let (rules, issues) = RuleSet::embedded(self.p.env());
        let issues: Vec<String> = issues
            .iter()
            .map(|i| format!("{} ({}): {}", i.rule_id, i.file, i.problem))
            .collect();
        if st.rules.is_none() || issues != st.rule_issues {
            for i in &issues {
                tracing::warn!("правило очищення {i}");
            }
        }
        st.rule_issues = issues;
        let rules = Arc::new(rules);
        st.rules = Some(rules.clone());
        rules
    }

    fn rules(&self) -> Arc<RuleSet> {
        let mut st = self.state();
        self.rules_in(&mut st, false)
    }

    fn tool_guard(&self) -> CleanResult<ToolGuard<'_>> {
        let mut st = self.state();
        if let Some(why) = st.busy() {
            return Err(failed(why));
        }
        st.tool_busy = true;
        Ok(ToolGuard(self))
    }

    /// Новий запуск програми — нова сесія: перед першою системною зміною знову точка відновлення.
    fn begin_session(&self, session: u64) {
        let mut st = self.state();
        if st.session != Some(session) {
            st.session = Some(session);
            st.restore = None;
        }
    }

    fn quarantine_root(&self, q: &QuarantineSpec) -> CleanResult<String> {
        match q
            .drive
            .as_deref()
            .and_then(|d| path::normalize(d).ok())
            .filter(|d| path::is_drive_root(d))
        {
            // На іншому диску — така сама захищена тека: `D:\OptimaX\<SID>\Quarantine`.
            Some(drive) => Ok(path::join(
                &self.p.dir_on_drive(&drive)?,
                QUARANTINE_DIR_NAME,
            )),
            None => Ok(path::join(&self.p.data_dir()?, QUARANTINE_DIR_NAME)),
        }
    }

    fn quarantine_index(&self) -> CleanResult<String> {
        Ok(path::join(&self.p.data_dir()?, "quarantine.json"))
    }

    fn journal_path(&self) -> CleanResult<String> {
        Ok(path::join(&self.p.data_dir()?, "cleanup-journal.json"))
    }

    fn journal(&self) -> CleanResult<CleanupJournal<'_>> {
        Ok(CleanupJournal::new(self.p.fs(), &self.journal_path()?))
    }

    fn open_quarantine(&self, q: &QuarantineSpec) -> CleanResult<Quarantine<'_>> {
        Ok(Quarantine::open(
            self.p.fs(),
            &self.quarantine_index()?,
            &self.quarantine_root(q)?,
            q.days,
        )?)
    }

    fn touches_system_drive(&self, plan: &CleanPlan) -> bool {
        path::drive_letter(&self.p.system_drive()).is_some_and(|d| plan.touches_drive(d))
    }

    /// Точка відновлення один раз за сесію перед системними змінами / очищенням системного диска.
    fn ensure_restore_point(&self, allow_without: bool) -> CleanResult<()> {
        if self.state().restore.is_some() {
            return Ok(());
        }
        let status = match self.p.restore().create(RESTORE_DESCRIPTION) {
            Ok(sequence) => RestorePointStatus::Created { sequence },
            Err(e) => {
                let reason = e.to_string();
                if !allow_without {
                    tracing::warn!("точку відновлення не створено ({reason}) — питаю користувача");
                    return Err(CleanError::RestorePoint(reason));
                }
                tracing::warn!("очищення без точки відновлення (за згодою користувача): {reason}");
                RestorePointStatus::SkippedByUser { reason }
            }
        };
        if let RestorePointStatus::Created { sequence } = &status {
            tracing::info!("створено точку відновлення №{sequence} перед очищенням");
        }
        self.state().restore = Some(status);
        Ok(())
    }

    /// Зупиняє скан і очищення після поточного файлу (DISM не переривається).
    pub fn cancel_all(&self) {
        let st = self.state();
        for t in st.scan.iter().chain(st.exec.iter()) {
            t.cancel();
        }
    }

    /// Зупиняє скан (звіт — за тим, що встигли побачити).
    pub fn cancel_scan(&self) {
        if let Some(t) = &self.state().scan {
            t.cancel();
        }
    }

    /// Зупиняє очищення після поточного файлу.
    pub fn cancel_exec(&self) {
        if let Some(t) = &self.state().exec {
            t.cancel();
        }
    }

    /// Правила, понижені до ⚪ (помилки бази).
    pub fn rule_issues(&self) -> Vec<String> {
        self.state().rule_issues.clone()
    }

    /// Останній звіт скану.
    pub fn last_scan(&self) -> Option<LastScan> {
        let st = self.state();
        Some(LastScan {
            report: st.report.clone()?,
            rule_issues: st.rule_issues.clone(),
        })
    }

    /// Займає модуль сканом диска `drive` (сам скан — [`ScanJob::run`]).
    pub fn start_scan(&self, drive: &str, full: bool, mode: ScanMode) -> CleanResult<ScanJob> {
        let root = path::normalize(drive)
            .ok()
            .filter(|r| path::is_drive_root(r))
            .ok_or_else(|| CleanError::Forbidden(format!("«{drive}» — не корінь диска")))?;
        let mut st = self.state();
        if let Some(why) = st.busy() {
            return Err(failed(why));
        }
        let keep = mode == ScanMode::Screen || st.report.is_none();
        let rules = match mode {
            // Новий скан екрана — нова база правил (теки користувача могли змінитися); план
            // попереднього скану більше не діє.
            ScanMode::Screen => {
                st.plan = None;
                self.rules_in(&mut st, true)
            }
            ScanMode::Background => self.rules_in(&mut st, false),
        };
        let cancel = CancelToken::new();
        st.scan = Some(cancel.clone());
        Ok(ScanJob {
            root,
            full,
            keep,
            cancel,
            rules,
        })
    }

    fn save_snapshot(&self, report: &ScanReport) {
        let (Ok(dir), Some(letter)) = (self.p.data_dir(), path::drive_letter(&report.drive)) else {
            return;
        };
        let snap = Snapshot {
            unix: report.started_unix,
            full: report.full,
            sizes: report.sizes.iter().cloned().collect(),
        };
        if let Err(e) = SnapshotStore::new(self.p.fs(), &dir, letter).push(snap) {
            tracing::warn!("не вдалося зберегти знімок розмірів: {e}");
        }
    }

    /// Сухий прогін: точний список файлів, сума й вердикти. Нічого не змінює. Вибрати можна лише
    /// об'єкти з останнього скану, позначені як доступні для очищення.
    pub fn dry_run(&self, paths: &[String], q: &QuarantineSpec) -> CleanResult<DryRunView> {
        let (rules, report, restore_done) = {
            let mut st = self.state();
            if let Some(why) = st.busy() {
                return Err(failed(why));
            }
            let report = st
                .report
                .clone()
                .ok_or_else(|| CleanError::NotFound("спершу проскануй диск".into()))?;
            (self.rules_in(&mut st, false), report, st.restore.is_some())
        };
        if paths.is_empty() {
            return Err(CleanError::Forbidden("нічого не вибрано".into()));
        }
        for p in paths {
            let ok = report
                .items
                .iter()
                .any(|i| i.selectable && path::eq_ci(&i.path, p));
            if !ok {
                return Err(CleanError::Forbidden(format!(
                    "{p}: не можна вибрати для очищення"
                )));
            }
        }
        let plan = build_plan(
            self.p.fs(),
            &rules,
            paths,
            self.p.now_unix(),
            &CancelToken::new(),
        )?;
        let qdir = self.quarantine_root(q)?;
        let qdrive = path::drive_letter(&qdir);
        let mut view = DryRunView {
            plan_id: 0,
            items: plan
                .items
                .iter()
                .map(|i| DryRunItem {
                    path: i.root.clone(),
                    rule_id: i.rule_id.clone(),
                    verdict: i.verdict,
                    action: i.action,
                    files: i.files.len() as u64,
                    bytes: i.bytes(),
                    sample: i
                        .files
                        .iter()
                        .take(DRY_RUN_SAMPLE)
                        .map(|f| f.path.clone())
                        .collect(),
                })
                .collect(),
            delete_bytes: plan.delete_bytes(),
            quarantine_bytes: plan.quarantine_bytes(),
            files: plan.file_count() as u64,
            skipped: plan.skipped.iter().take(SKIPPED_SAMPLE).cloned().collect(),
            skipped_total: plan.skipped.len() as u64,
            services: plan.services.clone(),
            quarantine_same_volume: plan
                .items
                .iter()
                .any(|i| i.action == Action::Quarantine && path::drive_letter(&i.root) == qdrive),
            quarantine_dir: qdir,
            quarantine_days: q.days.max(1),
            restore_point_pending: !restore_done && self.touches_system_drive(&plan),
        };
        let mut st = self.state();
        st.next_plan += 1;
        view.plan_id = st.next_plan;
        st.plan = Some((view.plan_id, Arc::new(plan)));
        Ok(view)
    }

    /// Займає модуль очищенням за показаним планом: запис у журналі й точка відновлення (якщо
    /// план чіпає системний диск). Не вдалося створити точку — `RestorePoint(причина)`; UI питає
    /// користувача й повторює з `allow_without_restore_point`. Саме очищення — [`ExecJob::run`].
    pub fn start_exec(
        &self,
        plan_id: u64,
        allow_without_restore_point: bool,
        session: u64,
        q: &QuarantineSpec,
    ) -> CleanResult<ExecJob> {
        self.begin_session(session);
        let plan = {
            let st = self.state();
            if let Some(why) = st.busy() {
                return Err(failed(why));
            }
            match &st.plan {
                Some((id, p)) if *id == plan_id => p.clone(),
                _ => {
                    return Err(CleanError::NotFound(format!(
                        "план #{plan_id}: зроби сухий прогін ще раз"
                    )))
                }
            }
        };
        // Запис у журнал — до будь-яких кроків: навіть невдала спроба має лишити слід.
        let journal = self.journal()?;
        let jid = journal.begin(RunKind::Clean, self.p.now_unix())?;
        tracing::info!(
            "очищення #{jid}: {} файлів за планом #{plan_id}; без точки відновлення: {}",
            plan.file_count(),
            if allow_without_restore_point {
                "дозволено"
            } else {
                "ні"
            }
        );
        let abort = |e: CleanError| {
            let msg = format!("не виконано: {e}");
            tracing::warn!("очищення #{jid} {msg}");
            let _ = journal.finish(jid, self.p.now_unix(), Vec::new(), |r| r.note = Some(msg));
            e
        };
        if self.touches_system_drive(&plan) {
            tracing::info!("очищення #{jid}: точка відновлення…");
            self.ensure_restore_point(allow_without_restore_point)
                .map_err(abort)?;
        }
        let mut st = self.state();
        if let Some(why) = st.busy() {
            return Err(abort(failed(why)));
        }
        // План виконується один раз.
        st.plan = None;
        let cancel = CancelToken::new();
        st.exec = Some(cancel.clone());
        let rules = self.rules_in(&mut st, false);
        Ok(ExecJob {
            jid,
            plan,
            rules,
            quarantine: q.clone(),
            cancel,
        })
    }

    /// Журнал очищення (найновіші — останні).
    pub fn journal_records(&self) -> CleanResult<Vec<JournalRecord>> {
        Ok(self.journal()?.load()?)
    }

    /// «Що виросло»: останній знімок диска порівняно зі знімком приблизно тижневої давності.
    pub fn growth(&self, drive: &str) -> CleanResult<Option<GrowthView>> {
        let letter = path::normalize(drive)
            .ok()
            .filter(|r| path::is_drive_root(r))
            .and_then(|r| path::drive_letter(&r))
            .ok_or_else(|| CleanError::Forbidden(format!("«{drive}» — не корінь диска")))?;
        let all = SnapshotStore::new(self.p.fs(), &self.p.data_dir()?, letter).load()?;
        let Some((current, history)) = all.split_last() else {
            return Ok(None);
        };
        let Some(base) = pick_baseline(history, current.unix, GROWTH_WINDOW_SECS) else {
            return Ok(None);
        };
        let rules = self.rules();
        let report = growth(base, current);
        Ok(Some(GrowthView {
            since_unix: report.since_unix,
            now_unix: report.now_unix,
            full: report.full,
            items: report
                .items
                .into_iter()
                .map(|g| {
                    let cls = rules.classify(&g.path);
                    GrowthItem {
                        verdict: cls.verdict,
                        rule_id: cls
                            .rule
                            .and_then(|i| rules.get(i))
                            .map(|r| r.rule.id.clone()),
                        growth: g,
                    }
                })
                .collect(),
        }))
    }

    /// Карантин: тека й активні партії.
    pub fn quarantine(&self, q: &QuarantineSpec) -> CleanResult<QuarantineState> {
        let dir = self.quarantine_root(q)?;
        let quarantine = self.open_quarantine(q)?;
        let now = self.p.now_unix();
        Ok(QuarantineState {
            on_system_drive: path::drive_letter(&dir) == path::drive_letter(&self.p.system_drive()),
            batches: quarantine
                .batches()
                .iter()
                .filter(|b| b.state == BatchState::Active)
                .map(|b| BatchView {
                    id: b.id.clone(),
                    created_unix: b.created_unix,
                    expires_unix: b.expires_unix,
                    bytes: b.bytes(),
                    files: b.entries.len() as u64,
                    same_volume: b.same_volume,
                    expired: b.expires_unix <= now,
                    dir: b.dir.clone(),
                })
                .collect(),
            dir,
        })
    }

    /// Відновлює (`restore`) або остаточно видаляє партію карантину.
    pub fn batch(&self, id: &str, restore: bool, q: &QuarantineSpec) -> CleanResult<JournalRecord> {
        let _guard = self.tool_guard()?;
        let rules = self.rules();
        let journal = self.journal()?;
        let mut quarantine = self.open_quarantine(q)?;
        let kind = if restore {
            RunKind::Restore
        } else {
            RunKind::Purge
        };
        let jid = journal.begin(kind, self.p.now_unix())?;
        let result = if restore {
            quarantine.restore(id, &rules)
        } else {
            quarantine.purge(id)
        };
        let out = match result {
            Ok(o) => o,
            Err(e) => {
                let msg = e.to_string();
                let _ = journal.finish(jid, self.p.now_unix(), Vec::new(), |r| r.note = Some(msg));
                return Err(e.into());
            }
        };
        let record = journal.finish(jid, self.p.now_unix(), out.lines, |r| {
            r.batch_id = Some(id.to_string());
            if restore {
                r.restored_bytes = out.bytes;
            } else {
                r.freed_bytes = out.bytes;
            }
        })?;
        tracing::info!(
            "карантин {id}: {} {} байтів",
            if restore {
                "відновлено"
            } else {
                "видалено остаточно"
            },
            out.bytes
        );
        Ok(record)
    }

    /// Видаляє партії з минулим терміном (кожна — окремим записом у журналі). Повертає, скільки
    /// видалено.
    pub fn auto_purge(&self, q: &QuarantineSpec) -> usize {
        let expired = self
            .open_quarantine(q)
            .map(|quarantine| quarantine.expired(self.p.now_unix()))
            .unwrap_or_default();
        let mut purged = 0;
        for id in expired {
            match self.batch(&id, false, q) {
                Ok(r) => {
                    purged += 1;
                    tracing::info!(
                        "автоочищення карантину: партія {id} видалена, звільнено {} байтів",
                        r.freed_bytes
                    );
                }
                Err(e) => tracing::warn!("автоочищення карантину: партія {id}: {e}"),
            }
        }
        purged
    }

    /// Штатне очищення компонентів Windows: `DISM /StartComponentCleanup`. Триває хвилини;
    /// звільнене місце — виміряна різниця вільного місця системного диска.
    pub fn dism(
        &self,
        allow_without_restore_point: bool,
        session: u64,
    ) -> CleanResult<JournalRecord> {
        self.begin_session(session);
        let _guard = self.tool_guard()?;
        self.ensure_restore_point(allow_without_restore_point)?;
        let sys = self.p.system_drive();
        let journal = self.journal()?;
        let jid = journal.begin(RunKind::Dism, self.p.now_unix())?;
        let before = self.p.free_bytes(&sys);
        tracing::info!("DISM /StartComponentCleanup: почато (може тривати кілька хвилин)");
        let out = self.p.tools().dism_component_cleanup();
        let after = self.p.free_bytes(&sys);
        let freed = match (before, after) {
            (Some(b), Some(a)) => a.saturating_sub(b),
            _ => 0,
        };
        let (ok, note) = match &out {
            Ok(o) => (o.ok(), format!("код завершення {}\n{}", o.code, o.output)),
            Err(e) => (false, e.to_string()),
        };
        let record = journal.finish(jid, self.p.now_unix(), Vec::new(), |r| {
            r.freed_bytes = freed;
            r.note = Some(note.clone());
        })?;
        if ok {
            tracing::info!("DISM завершено, вільного місця більше на {freed} байтів (виміряно)");
            Ok(record)
        } else {
            tracing::error!("DISM завершився з помилкою: {note}");
            Err(failed(format!("DISM завершився з помилкою: {note}")))
        }
    }

    /// Вимикає (`enable = false`) або вмикає гібернацію через `powercfg`. Вимкнення — системна
    /// зміна: перед нею точка відновлення; повернути — тією ж командою з `enable = true`.
    pub fn hibernate(
        &self,
        enable: bool,
        allow_without_restore_point: bool,
        session: u64,
    ) -> CleanResult<JournalRecord> {
        self.begin_session(session);
        let _guard = self.tool_guard()?;
        if !enable {
            self.ensure_restore_point(allow_without_restore_point)?;
        }
        let fs = self.p.fs();
        let hiberfil = path::join(&self.p.system_drive(), "hiberfil.sys");
        let size = || fs.metadata(&hiberfil).ok().map(|m| m.len);
        let before = size();
        let journal = self.journal()?;
        let kind = if enable {
            RunKind::HibernateOn
        } else {
            RunKind::HibernateOff
        };
        let jid = journal.begin(kind, self.p.now_unix())?;
        let out = self.p.tools().set_hibernate(enable);
        let after = size();
        let (ok, note) = match &out {
            Ok(o) => (
                o.ok(),
                format!(
                    "powercfg /hibernate {}: код {}\n{}",
                    if enable { "on" } else { "off" },
                    o.code,
                    o.output
                ),
            ),
            Err(e) => (false, e.to_string()),
        };
        let freed = match (before, after) {
            (Some(b), None) => b,
            _ => 0,
        };
        let record = journal.finish(jid, self.p.now_unix(), Vec::new(), |r| {
            r.freed_bytes = freed;
            r.note = Some(note.clone());
        })?;
        if ok {
            tracing::info!(
                "гібернацію {}; hiberfil.sys: {}",
                if enable {
                    "увімкнено"
                } else {
                    "вимкнено"
                },
                if after.is_some() { "є" } else { "немає" }
            );
            Ok(record)
        } else {
            Err(failed(note))
        }
    }
}

impl ScanJob {
    /// Звіт стане звітом екрана «Очищення».
    pub fn keeps(&self) -> bool {
        self.keep
    }

    /// Не вдалося запустити потік скану — звільняє модуль.
    pub fn abandon(self, cleaner: &Cleaner) {
        cleaner.state().scan = None;
    }

    /// Сканує диск (блокує потік). Проміжні звіти — лише для показу і лише якщо звіт стане
    /// звітом екрана: вибір і сухий прогін ідуть за остаточним.
    pub fn run(
        self,
        cleaner: &Cleaner,
        progress: &(dyn Fn(ScanProgress) + Sync),
        partial: &dyn Fn(&ScanReport),
    ) -> ScanReport {
        let threads = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).clamp(1, 6))
            .unwrap_or(2);
        let root = &self.root;
        tracing::info!(
            "скан {root} ({})",
            if self.full {
                "повний"
            } else {
                "за правилами"
            }
        );
        let opts = ScanOptions {
            drive: root.clone(),
            full: self.full,
            now_unix: cleaner.p.now_unix(),
            threads,
        };
        let keep = self.keep;
        let report = scan(
            cleaner.p.fs(),
            &self.rules,
            &opts,
            &self.cancel,
            progress,
            &|r| {
                if keep {
                    partial(r);
                }
            },
        );
        if !report.cancelled {
            cleaner.save_snapshot(&report);
        }
        tracing::info!(
            "скан {root}: {} об'єктів за правилами, {} невідомих, {} мс{}",
            report.items.len(),
            report.unknown.len(),
            report.duration_ms,
            if report.cancelled {
                " (скасовано)"
            } else {
                ""
            }
        );
        let mut st = cleaner.state();
        if keep {
            st.report = Some(report.clone());
        }
        st.scan = None;
        report
    }
}

impl ExecJob {
    /// Не вдалося запустити потік очищення — запис у журналі й звільнення модуля.
    pub fn abandon(self, cleaner: &Cleaner, why: &str) {
        cleaner.state().exec = None;
        if let Ok(journal) = cleaner.journal() {
            let note = format!("не виконано: {why}");
            let _ = journal.finish(self.jid, cleaner.p.now_unix(), Vec::new(), |r| {
                r.note = Some(note)
            });
        }
    }

    /// Очищує за планом (блокує потік). Модуль звільняється до повернення: UI одразу запускає
    /// новий скан.
    pub fn run(self, cleaner: &Cleaner, progress: &dyn Fn(ExecProgress)) -> CleanResult<ExecView> {
        let r = self.clean(cleaner, progress);
        cleaner.state().exec = None;
        if let Err(e) = &r {
            tracing::error!("очищення не вдалося: {e}");
        }
        r
    }

    fn clean(&self, cleaner: &Cleaner, progress: &dyn Fn(ExecProgress)) -> CleanResult<ExecView> {
        let p = &cleaner.p;
        let journal = cleaner.journal()?;
        let jid = self.jid;
        let needs_q = self
            .plan
            .items
            .iter()
            .any(|i| i.action == Action::Quarantine);
        let opened = if needs_q {
            cleaner.open_quarantine(&self.quarantine).map(Some)
        } else {
            Ok(None)
        };
        let mut quarantine = match opened {
            Ok(q) => q,
            Err(e) => {
                let msg = e.to_string();
                let _ = journal.finish(jid, p.now_unix(), Vec::new(), |r| r.note = Some(msg));
                return Err(e);
            }
        };
        let ctx = ExecContext {
            fs: p.fs(),
            rules: &self.rules,
            services: Some(p.services()),
            now_unix: p.now_unix(),
            cancel: &self.cancel,
        };
        let res = match execute(&ctx, &self.plan, quarantine.as_mut(), progress) {
            Ok(r) => r,
            Err(e) => {
                let msg = e.to_string();
                let _ = journal.finish(jid, p.now_unix(), Vec::new(), |r| r.note = Some(msg));
                return Err(e.into());
            }
        };
        let same_volume = res
            .batch_id
            .as_ref()
            .and_then(|id| quarantine.as_ref()?.batches().iter().find(|b| &b.id == id))
            .is_some_and(|b| b.same_volume);
        let services_note = res
            .services
            .iter()
            .map(|s| {
                format!(
                    "служба {}: {}{}",
                    s.name,
                    if s.was_running {
                        if s.restored {
                            "зупинялась, запущена знову"
                        } else {
                            "зупинялась, НЕ запущена знову"
                        }
                    } else {
                        "не працювала, не чіпали"
                    },
                    s.error
                        .as_deref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let record = journal.finish(jid, p.now_unix(), res.lines.clone(), |r| {
            r.freed_bytes = res.freed_bytes;
            r.quarantined_bytes = res.quarantined_bytes;
            r.batch_id = res.batch_id.clone();
            r.note = (!services_note.is_empty()).then_some(services_note);
        })?;
        tracing::info!(
            "очищення: видалено {} файлів ({} байтів), у карантин {} файлів ({} байтів), пропущено {}{}",
            res.deleted_files,
            res.freed_bytes,
            res.quarantined_files,
            res.quarantined_bytes,
            res.skipped.len(),
            if res.cancelled { ", скасовано" } else { "" }
        );
        Ok(ExecView {
            record,
            services: res.services,
            quarantine_same_volume: same_volume,
            cancelled: res.cancelled,
        })
    }
}

// ---- Служба ----

/// Очищення одного користувача в службі: [`Cleaner`] і хід фонових операцій для опитування.
struct UserCleanup {
    cleaner: Arc<Cleaner>,
    events: Arc<Mutex<CleanupEvents>>,
}

fn events(e: &Mutex<CleanupEvents>) -> MutexGuard<'_, CleanupEvents> {
    e.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Створює [`Cleaner`] для користувача з цим SID.
pub type CleanerFactory = Box<dyn Fn(&str) -> Result<Cleaner, String> + Send + Sync>;

/// Очищення в службі для всіх користувачів, що під'єднувались.
pub struct CleanupHost {
    users: Mutex<HashMap<String, Arc<UserCleanup>>>,
    make: CleanerFactory,
}

fn reply<T: Serialize>(r: &CleanResult<T>) -> Reply {
    serde_json::to_value(r).map_or_else(|e| Reply::Error(e.to_string()), Reply::Json)
}

fn spec(drive: &Option<String>, days: u32) -> QuarantineSpec {
    QuarantineSpec {
        drive: drive.clone(),
        days,
    }
}

impl CleanupHost {
    /// `make` — [`Cleaner`] для SID (служба: теки й файли цього користувача).
    pub fn new(make: CleanerFactory) -> Self {
        Self {
            users: Mutex::new(HashMap::new()),
            make,
        }
    }

    fn user(&self, sid: &str) -> Result<Arc<UserCleanup>, String> {
        let mut users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(u) = users.get(sid) {
            return Ok(u.clone());
        }
        let cleaner = (self.make)(sid)?;
        tracing::info!("служба: очищення для {sid}");
        let u = Arc::new(UserCleanup {
            cleaner: Arc::new(cleaner),
            events: Arc::new(Mutex::new(CleanupEvents::default())),
        });
        users.insert(sid.to_string(), u.clone());
        Ok(u)
    }

    /// Виконує команду очищення від клієнта (`pid`, `sid` — з токена його процесу); `None` —
    /// команда не про очищення. Довгі дії (скан, очищення) ідуть в окремому потоці, хід —
    /// [`Command::CleanupPoll`].
    pub fn handle(&self, pid: u32, sid: &str, cmd: &Command) -> Option<Reply> {
        if !is_cleanup(cmd) {
            return None;
        }
        let u = match self.user(sid) {
            Ok(u) => u,
            Err(e) => return Some(Reply::Error(format!("очищення недоступне: {e}"))),
        };
        let c = &u.cleaner;
        c.set_client(pid);
        Some(match cmd {
            Command::CleanupScanStart {
                drive,
                full,
                background,
            } => {
                let mode = if *background {
                    ScanMode::Background
                } else {
                    ScanMode::Screen
                };
                reply(&self.scan(&u, drive, *full, mode))
            }
            Command::CleanupScanCancel => {
                c.cancel_scan();
                reply(&Ok(()))
            }
            Command::CleanupPoll => reply(&Ok(std::mem::take(&mut *events(&u.events)))),
            Command::CleanupLastScan => reply(&Ok(c.last_scan())),
            Command::CleanupDryRun {
                paths,
                quarantine_drive,
                quarantine_days,
            } => reply(&c.dry_run(paths, &spec(quarantine_drive, *quarantine_days))),
            Command::CleanupExecute {
                plan_id,
                allow_without_restore_point,
                session,
                quarantine_drive,
                quarantine_days,
            } => reply(&self.exec(
                &u,
                *plan_id,
                *allow_without_restore_point,
                *session,
                &spec(quarantine_drive, *quarantine_days),
            )),
            Command::CleanupExecCancel => {
                c.cancel_exec();
                reply(&Ok(()))
            }
            Command::CleanupJournal => reply(&c.journal_records()),
            Command::CleanupGrowth { drive } => reply(&c.growth(drive)),
            Command::CleanupQuarantine {
                quarantine_drive,
                quarantine_days,
            } => reply(&c.quarantine(&spec(quarantine_drive, *quarantine_days))),
            Command::CleanupBatch {
                id,
                restore,
                quarantine_drive,
                quarantine_days,
            } => reply(&c.batch(id, *restore, &spec(quarantine_drive, *quarantine_days))),
            Command::CleanupAutoPurge {
                quarantine_drive,
                quarantine_days,
            } => reply(&Ok(c.auto_purge(&spec(quarantine_drive, *quarantine_days)))),
            Command::CleanupDism {
                allow_without_restore_point,
                session,
            } => reply(&c.dism(*allow_without_restore_point, *session)),
            Command::CleanupHibernate {
                enable,
                allow_without_restore_point,
                session,
            } => reply(&c.hibernate(*enable, *allow_without_restore_point, *session)),
            _ => return None,
        })
    }

    fn scan(
        &self,
        u: &Arc<UserCleanup>,
        drive: &str,
        full: bool,
        mode: ScanMode,
    ) -> CleanResult<()> {
        let job = u.cleaner.start_scan(drive, full, mode)?;
        {
            let mut e = events(&u.events);
            e.scan_progress = None;
            e.scan_partial = None;
            e.scan_done = None;
        }
        let (cleaner, ev) = (u.cleaner.clone(), u.events.clone());
        let spawned = std::thread::Builder::new()
            .name("optimax-svc-cleanup-scan".into())
            .spawn(move || {
                let kept = job.keeps();
                let issues = || cleaner.rule_issues();
                let report = job.run(&cleaner, &|p| events(&ev).scan_progress = Some(p), &|r| {
                    events(&ev).scan_partial = Some(LastScan {
                        report: r.clone(),
                        rule_issues: issues(),
                    });
                });
                events(&ev).scan_done = Some(ScanDone {
                    scan: LastScan {
                        report,
                        rule_issues: issues(),
                    },
                    kept,
                });
            });
        spawned.map(|_| ()).map_err(|e| {
            u.cleaner.state().scan = None;
            failed(e)
        })
    }

    fn exec(
        &self,
        u: &Arc<UserCleanup>,
        plan_id: u64,
        allow: bool,
        session: u64,
        q: &QuarantineSpec,
    ) -> CleanResult<()> {
        let job = u.cleaner.start_exec(plan_id, allow, session, q)?;
        {
            let mut e = events(&u.events);
            e.exec_progress = None;
            e.exec_done = None;
        }
        let (cleaner, ev) = (u.cleaner.clone(), u.events.clone());
        // Потік може не запуститися — тоді план і журнал треба закрити тут.
        let slot = Arc::new(Mutex::new(Some(job)));
        let in_thread = slot.clone();
        let spawned = std::thread::Builder::new()
            .name("optimax-svc-cleanup-exec".into())
            .spawn(move || {
                let job = in_thread
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(job) = job {
                    let done = job.run(&cleaner, &|p| events(&ev).exec_progress = Some(p));
                    events(&ev).exec_done = Some(done);
                }
            });
        match spawned {
            Ok(_) => Ok(()),
            Err(e) => {
                let job = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
                if let Some(job) = job {
                    job.abandon(&u.cleaner, &e.to_string());
                }
                Err(failed(e))
            }
        }
    }
}

fn is_cleanup(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::CleanupScanStart { .. }
            | Command::CleanupScanCancel
            | Command::CleanupPoll
            | Command::CleanupLastScan
            | Command::CleanupDryRun { .. }
            | Command::CleanupExecute { .. }
            | Command::CleanupExecCancel
            | Command::CleanupJournal
            | Command::CleanupGrowth { .. }
            | Command::CleanupQuarantine { .. }
            | Command::CleanupBatch { .. }
            | Command::CleanupAutoPurge { .. }
            | Command::CleanupDism { .. }
            | Command::CleanupHibernate { .. }
    )
}

#[cfg(windows)]
pub use self::windows::WinPlatform;

#[cfg(windows)]
mod windows {
    use super::{failed, CleanResult, CleanupPlatform};
    use optimax_cleanup::system::{SystemTools, WinTools};
    use optimax_cleanup::{path, Env, FsApi, StdFs};
    use optimax_core::datadir;
    use optimax_core::service::{ServiceApi, WinServices};
    use optimax_tweaks::restore::{RestorePointApi, WinRestorePoint};
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;

    static FS: StdFs = StdFs;
    static SERVICES: WinServices = WinServices;
    static RESTORE: WinRestorePoint = WinRestorePoint;
    static TOOLS: WinTools = WinTools;

    /// Справжня система. Програма — свої теки ([`WinPlatform::local`]); служба — теки клієнта
    /// ([`WinPlatform::for_user`]).
    pub struct WinPlatform {
        /// SID клієнта служби (`None` — користувач цього процесу).
        sid: Option<String>,
        /// PID останнього клієнта (теки правил — його користувача).
        pid: AtomicU32,
        data_dir: OnceLock<String>,
    }

    impl WinPlatform {
        /// Програма без служби: теки й дані користувача цього процесу.
        pub fn local() -> Self {
            Self {
                sid: None,
                pid: AtomicU32::new(0),
                data_dir: OnceLock::new(),
            }
        }

        /// Служба: теки й дані користувача з цим SID (з токена клієнта).
        pub fn for_user(sid: &str) -> Result<Self, String> {
            if !datadir::is_plain_sid(sid) {
                return Err(format!("неочікуваний SID: {sid}"));
            }
            Ok(Self {
                sid: Some(sid.to_string()),
                pid: AtomicU32::new(0),
                data_dir: OnceLock::new(),
            })
        }
    }

    fn norm(p: &Path) -> CleanResult<String> {
        path::normalize(&p.to_string_lossy()).map_err(failed)
    }

    impl CleanupPlatform for WinPlatform {
        fn env(&self) -> Env {
            match (&self.sid, self.pid.load(Ordering::Relaxed)) {
                (None, _) => Env::from_system(),
                (Some(_), pid) => Env::for_process(pid),
            }
        }

        fn data_dir(&self) -> CleanResult<String> {
            if let Some(d) = self.data_dir.get() {
                return Ok(d.clone());
            }
            let dir = match &self.sid {
                Some(sid) => norm(&datadir::data_dir_for(sid).map_err(failed)?)?,
                None => norm(datadir::data_dir().map_err(failed)?)?,
            };
            Ok(self.data_dir.get_or_init(|| dir).clone())
        }

        fn dir_on_drive(&self, drive: &str) -> CleanResult<String> {
            let base = Path::new(drive);
            let dir = match &self.sid {
                Some(sid) => datadir::secure_dir_of(base, sid),
                None => datadir::secure_dir_on(base),
            };
            norm(&dir.map_err(failed)?)
        }

        fn system_drive(&self) -> String {
            optimax_core::sysdirs::system_drive()
                .and_then(|d| path::normalize(&format!("{d}\\")).ok())
                .unwrap_or_else(|| r"C:\".to_string())
        }

        fn free_bytes(&self, drive: &str) -> Option<u64> {
            let disks = sysinfo::Disks::new_with_refreshed_list();
            disks
                .list()
                .iter()
                .find(|d| {
                    d.mount_point()
                        .to_str()
                        .and_then(|m| path::normalize(m).ok())
                        .is_some_and(|m| path::eq_ci(&m, drive))
                })
                .map(|d| d.available_space())
        }

        fn now_unix(&self) -> i64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_secs()).ok())
                .unwrap_or(0)
        }

        fn fs(&self) -> &dyn FsApi {
            &FS
        }

        fn services(&self) -> &dyn ServiceApi {
            &SERVICES
        }

        fn restore(&self) -> &dyn RestorePointApi {
            &RESTORE
        }

        fn tools(&self) -> &dyn SystemTools {
            &TOOLS
        }

        fn set_client(&self, pid: u32) {
            self.pid.store(pid, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests;
