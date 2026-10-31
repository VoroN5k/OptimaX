//! Очищення диска: скан (`cleanup-progress`, `cleanup-scan-partial`, `cleanup-scan-done`,
//! `cleanup-error`), сухий прогін,
//! очищення (`cleanup-exec-progress`, `cleanup-exec-done`), карантин, журнал, «Що виросло» і
//! штатні засоби Windows (DISM, powercfg). Вердикти визначає лише бекенд.

use super::blocking;
use crate::error::{CmdError, CmdResult};
use crate::settings::CleanupSettings;
use crate::state::{lock, AppState, RESTORE, SERVICES};
use optimax_cleanup::journal::{CleanupJournal, JournalRecord, RunKind};
use optimax_cleanup::plan::{Action, ServiceNote, Skipped};
use optimax_cleanup::quarantine::{BatchState, Quarantine, QUARANTINE_DIR_NAME};
use optimax_cleanup::rules::{RuleSet, Verdict};
use optimax_cleanup::scan::{ScanOptions, ScanReport};
use optimax_cleanup::snapshot::{growth, pick_baseline, Growth, Snapshot, SnapshotStore};
use optimax_cleanup::system::{SystemTools, WinTools};
use optimax_cleanup::{
    build_plan, execute, path, scan, CancelToken, CleanPlan, Env, ExecContext, FsApi, StdFs,
};
use optimax_tweaks::restore::{RestorePointApi, RestorePointStatus};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

/// Опис точки відновлення для очищення.
const RESTORE_DESCRIPTION: &str = "OptimaX: очищення диска";
/// «Що виросло» — порівняння зі сканом приблизно тижневої давності.
const GROWTH_WINDOW_SECS: i64 = 7 * 86_400;
/// Скільки файлів показувати в сухому прогоні для кожного об'єкта.
const DRY_RUN_SAMPLE: usize = 200;
/// Скільки пропущених показувати.
const SKIPPED_SAMPLE: usize = 200;

/// Стан модуля очищення.
#[derive(Default)]
pub struct CleanupState {
    rules: Option<Arc<RuleSet>>,
    rule_issues: Vec<String>,
    scan: Option<CancelToken>,
    exec: Option<CancelToken>,
    tool_busy: bool,
    report: Option<ScanReport>,
    plan: Option<(u64, Arc<CleanPlan>)>,
    next_plan: u64,
    restore: Option<RestorePointStatus>,
}

impl CleanupState {
    /// База правил (завантажується один раз; проблеми валідації — у журнал).
    fn rules(&mut self) -> Arc<RuleSet> {
        if let Some(r) = &self.rules {
            return r.clone();
        }
        let (rules, issues) = RuleSet::embedded(Env::from_system());
        for i in &issues {
            tracing::warn!("правило очищення {} ({}): {}", i.rule_id, i.file, i.problem);
        }
        self.rule_issues = issues
            .iter()
            .map(|i| format!("{} ({}): {}", i.rule_id, i.file, i.problem))
            .collect();
        let rules = Arc::new(rules);
        self.rules = Some(rules.clone());
        rules
    }

    fn cancel(&self) {
        for t in self.scan.iter().chain(self.exec.iter()) {
            t.cancel();
        }
    }

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

/// Диск.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DriveView {
    /// Корінь (`C:\`).
    pub root: String,
    /// Мітка тому.
    pub label: String,
    /// Обсяг.
    pub total_bytes: u64,
    /// Вільно.
    pub free_bytes: u64,
    /// Системний диск.
    pub is_system: bool,
    /// Знімний.
    pub removable: bool,
}

/// Результат скану для екрана.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScanView {
    /// Звіт.
    pub report: ScanReport,
    /// Диск (на момент завершення скану).
    pub drive: Option<DriveView>,
    /// 🟢: виміряно байтів, що підпадають під правила.
    pub safe_bytes: u64,
    /// 🟡: виміряно байтів (лише через карантин).
    pub consequences_bytes: u64,
    /// Мало місця, а чищення дає мало — порадити перенести ігри/теки (лише порада).
    pub suggest_move: bool,
    /// Правила, понижені до ⚪ (помилки бази).
    pub rule_issues: Vec<String>,
}

/// Об'єкт сухого прогону.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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

/// Карантин.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct QuarantineView {
    /// Поточна тека карантину.
    pub dir: String,
    /// Налаштування.
    pub settings: CleanupSettings,
    /// Карантин на системному диску.
    pub on_system_drive: bool,
    /// Активні партії.
    pub batches: Vec<BatchView>,
    /// Інші диски, придатні для карантину.
    pub other_drives: Vec<DriveView>,
}

/// Рядок «Що виросло».
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct GrowthItem {
    /// Зміна.
    pub growth: Growth,
    /// Вердикт для шляху.
    pub verdict: Verdict,
    /// Правило.
    pub rule_id: Option<String>,
}

/// Що виросло.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

/// Захищена тека даних (`%ProgramData%\OptimaX\<SID>`).
pub(crate) fn data_dir() -> CmdResult<String> {
    let dir = optimax_core::datadir::data_dir().map_err(failed)?;
    path::normalize(&dir.to_string_lossy()).map_err(failed)
}

fn system_drive() -> String {
    optimax_core::sysdirs::system_drive()
        .and_then(|d| path::normalize(&format!("{d}\\")).ok())
        .unwrap_or_else(|| r"C:\".to_string())
}

fn quarantine_root(settings: &CleanupSettings) -> CmdResult<String> {
    match settings
        .quarantine_drive
        .as_deref()
        .and_then(|d| path::normalize(d).ok())
        .filter(|d| path::is_drive_root(d))
    {
        // На іншому диску — така сама захищена тека: `D:\OptimaX\<SID>\Quarantine`.
        Some(drive) => {
            let dir = optimax_core::datadir::secure_dir_on(std::path::Path::new(&drive))
                .map_err(failed)?;
            let dir = path::normalize(&dir.to_string_lossy()).map_err(failed)?;
            Ok(path::join(&dir, QUARANTINE_DIR_NAME))
        }
        None => Ok(path::join(&data_dir()?, QUARANTINE_DIR_NAME)),
    }
}

fn quarantine_index() -> CmdResult<String> {
    Ok(path::join(&data_dir()?, "quarantine.json"))
}

pub(crate) fn journal_path() -> CmdResult<String> {
    Ok(path::join(&data_dir()?, "cleanup-journal.json"))
}

fn list_drives() -> Vec<DriveView> {
    let sys = system_drive();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<DriveView> = Vec::new();
    for d in disks.list() {
        let Some(root) = d
            .mount_point()
            .to_str()
            .and_then(|m| path::normalize(m).ok())
            .filter(|r| path::is_drive_root(r))
        else {
            continue;
        };
        if out.iter().any(|o| path::eq_ci(&o.root, &root)) {
            continue;
        }
        out.push(DriveView {
            is_system: path::eq_ci(&root, &sys),
            label: d.name().to_string_lossy().into_owned(),
            total_bytes: d.total_space(),
            free_bytes: d.available_space(),
            removable: d.is_removable(),
            root,
        });
    }
    out.sort_by(|a, b| b.is_system.cmp(&a.is_system).then(a.root.cmp(&b.root)));
    out
}

fn drive_view(root: &str) -> Option<DriveView> {
    list_drives()
        .into_iter()
        .find(|d| path::eq_ci(&d.root, root))
}

/// Точка відновлення один раз за сесію перед системними змінами / очищенням системного диска.
fn ensure_restore_point(app: &AppHandle, allow_without: bool) -> CmdResult<()> {
    let state = app.state::<AppState>();
    if lock(&state.cleanup).restore.is_some() {
        return Ok(());
    }
    let status = match RESTORE.create(RESTORE_DESCRIPTION) {
        Ok(sequence) => RestorePointStatus::Created { sequence },
        Err(e) => {
            let reason = e.to_string();
            if !allow_without {
                tracing::warn!("точку відновлення не створено ({reason}) — питаю користувача");
                return Err(CmdError::RestorePoint(reason));
            }
            tracing::warn!("очищення без точки відновлення (за згодою користувача): {reason}");
            RestorePointStatus::SkippedByUser { reason }
        }
    };
    if let RestorePointStatus::Created { sequence } = &status {
        tracing::info!("створено точку відновлення №{sequence} перед очищенням");
    }
    lock(&state.cleanup).restore = Some(status);
    Ok(())
}

/// Знімає «зайнято» з модуля при виході з області видимості (навіть після помилки).
struct ToolGuard(AppHandle);

impl ToolGuard {
    fn acquire(app: &AppHandle) -> CmdResult<Self> {
        let state = app.state::<AppState>();
        let mut c = lock(&state.cleanup);
        if let Some(why) = c.busy() {
            return Err(failed(why));
        }
        c.tool_busy = true;
        Ok(Self(app.clone()))
    }
}

impl Drop for ToolGuard {
    fn drop(&mut self) {
        lock(&self.0.state::<AppState>().cleanup).tool_busy = false;
    }
}

/// При виході: зупиняє скан і очищення після поточного файлу (DISM не переривається).
pub fn cancel_all(state: &CleanupState) {
    state.cancel();
}

/// Диски.
#[tauri::command]
pub async fn cleanup_drives() -> CmdResult<Vec<DriveView>> {
    blocking(|| Ok(list_drives())).await
}

/// Останній звіт скану цієї сесії (щоб екран не сканував заново після переходу між вкладками).
#[tauri::command]
pub fn cleanup_last_scan(state: State<'_, AppState>) -> Option<ScanView> {
    let c = lock(&state.cleanup);
    let issues = c.rule_issues.clone();
    let report = c.report.clone()?;
    drop(c);
    Some(make_view(report, issues))
}

fn make_view(report: ScanReport, rule_issues: Vec<String>) -> ScanView {
    let sum = |v: Verdict| {
        report
            .items
            .iter()
            .filter(|i| i.selectable && i.verdict == v)
            .map(|i| i.eligible_bytes)
            .sum::<u64>()
    };
    let (safe_bytes, consequences_bytes) = (sum(Verdict::Safe), sum(Verdict::Consequences));
    let drive = drive_view(&report.drive);
    let suggest_move = drive.as_ref().is_some_and(|d| {
        const GIB: u64 = 1 << 30;
        let low = d.total_bytes > 0 && d.free_bytes.saturating_mul(100) < d.total_bytes * 15;
        let little = safe_bytes + consequences_bytes < (5 * GIB).max(d.total_bytes / 20);
        low && little
    });
    ScanView {
        report,
        drive,
        safe_bytes,
        consequences_bytes,
        suggest_move,
        rule_issues,
    }
}

/// Починає скан диска. `full` — обхід усього диска (довше; показує «Невідоме» й дає повний знімок).
#[tauri::command]
pub fn cleanup_scan_start(
    app: AppHandle,
    state: State<'_, AppState>,
    drive: String,
    full: bool,
) -> CmdResult<()> {
    let root = path::normalize(&drive)
        .ok()
        .filter(|r| path::is_drive_root(r))
        .ok_or_else(|| CmdError::Forbidden(format!("«{drive}» — не корінь диска")))?;
    let (cancel, rules, issues) = {
        let mut c = lock(&state.cleanup);
        if let Some(why) = c.busy() {
            return Err(failed(why));
        }
        let t = CancelToken::new();
        c.scan = Some(t.clone());
        c.plan = None;
        let rules = c.rules();
        (t, rules, c.rule_issues.clone())
    };
    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(1, 6))
        .unwrap_or(2);
    let spawned = std::thread::Builder::new()
        .name("optimax-cleanup-scan".into())
        .spawn(move || {
            tracing::info!(
                "скан {root} ({})",
                if full {
                    "повний"
                } else {
                    "за правилами"
                }
            );
            let opts = ScanOptions {
                drive: root.clone(),
                full,
                now_unix: now_unix(),
                threads,
            };
            // Проміжний звіт лише для показу: вибір і сухий прогін ідуть за остаточним.
            let report = scan(
                &StdFs,
                &rules,
                &opts,
                &cancel,
                &|p| {
                    let _ = app.emit("cleanup-progress", p);
                },
                &|r| {
                    let _ = app.emit("cleanup-scan-partial", make_view(r.clone(), issues.clone()));
                },
            );
            if !report.cancelled {
                save_snapshot(&report);
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
            let state = app.state::<AppState>();
            let issues = {
                let mut c = lock(&state.cleanup);
                c.report = Some(report.clone());
                c.scan = None;
                c.rule_issues.clone()
            };
            let _ = app.emit("cleanup-scan-done", make_view(report, issues));
        });
    if let Err(e) = spawned {
        lock(&state.cleanup).scan = None;
        return Err(failed(e));
    }
    Ok(())
}

fn save_snapshot(report: &ScanReport) {
    let (Ok(dir), Some(letter)) = (data_dir(), path::drive_letter(&report.drive)) else {
        return;
    };
    let snap = Snapshot {
        unix: report.started_unix,
        full: report.full,
        sizes: report.sizes.iter().cloned().collect(),
    };
    if let Err(e) = SnapshotStore::new(&StdFs, &dir, letter).push(snap) {
        tracing::warn!("не вдалося зберегти знімок розмірів: {e}");
    }
}

/// Зупиняє скан (звіт — за тим, що встигли побачити).
#[tauri::command]
pub fn cleanup_scan_cancel(state: State<'_, AppState>) {
    if let Some(c) = lock(&state.cleanup).scan.as_ref() {
        c.cancel();
    }
}

/// Сухий прогін: точний список файлів, сума й вердикти. Нічого не змінює.
#[tauri::command]
pub async fn cleanup_dry_run(app: AppHandle, paths: Vec<String>) -> CmdResult<DryRunView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (rules, report, restore_done) = {
            let mut c = lock(&state.cleanup);
            if let Some(why) = c.busy() {
                return Err(failed(why));
            }
            let report = c
                .report
                .clone()
                .ok_or_else(|| CmdError::NotFound("спершу проскануй диск".into()))?;
            (c.rules(), report, c.restore.is_some())
        };
        if paths.is_empty() {
            return Err(CmdError::Forbidden("нічого не вибрано".into()));
        }
        // Вибрати можна лише об'єкти з останнього скану, позначені як доступні для очищення.
        for p in &paths {
            let ok = report
                .items
                .iter()
                .any(|i| i.selectable && path::eq_ci(&i.path, p));
            if !ok {
                return Err(CmdError::Forbidden(format!(
                    "{p}: не можна вибрати для очищення"
                )));
            }
        }
        let plan = build_plan(&StdFs, &rules, &paths, now_unix(), &CancelToken::new())?;
        let settings = lock(&state.settings).cleanup.clone();
        let qdir = quarantine_root(&settings)?;
        let qdrive = path::drive_letter(&qdir);
        let view = DryRunView {
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
            quarantine_days: settings.quarantine_days,
            restore_point_pending: !restore_done
                && path::drive_letter(&system_drive()).is_some_and(|d| plan.touches_drive(d)),
        };
        let mut c = lock(&state.cleanup);
        c.next_plan += 1;
        let id = c.next_plan;
        c.plan = Some((id, Arc::new(plan)));
        Ok(DryRunView {
            plan_id: id,
            ..view
        })
    })
    .await
}

/// Виконує показаний план. Якщо точку відновлення не створено, повертає `RestorePoint(причина)`;
/// UI питає користувача й повторює з `allow_without_restore_point`.
#[tauri::command]
pub async fn cleanup_execute(
    app: AppHandle,
    plan_id: u64,
    allow_without_restore_point: bool,
) -> CmdResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        let plan = {
            let c = lock(&state.cleanup);
            if let Some(why) = c.busy() {
                return Err(failed(why));
            }
            match &c.plan {
                Some((id, p)) if *id == plan_id => p.clone(),
                _ => {
                    return Err(CmdError::NotFound(format!(
                        "план #{plan_id}: зроби сухий прогін ще раз"
                    )))
                }
            }
        };
        // Запис у журнал — до будь-яких кроків: навіть невдала спроба має лишити слід.
        let journal = CleanupJournal::new(&StdFs, &journal_path()?);
        let jid = journal.begin(RunKind::Clean, now_unix())?;
        tracing::info!(
            "очищення #{jid}: {} файлів за планом #{plan_id}; без точки відновлення: {}",
            plan.file_count(),
            if allow_without_restore_point {
                "дозволено"
            } else {
                "ні"
            }
        );
        let abort = |e: CmdError| {
            let msg = format!("не виконано: {}", e.message());
            tracing::warn!("очищення #{jid} {msg}");
            let _ = journal.finish(jid, now_unix(), Vec::new(), |r| r.note = Some(msg));
            e
        };
        if path::drive_letter(&system_drive()).is_some_and(|d| plan.touches_drive(d)) {
            tracing::info!("очищення #{jid}: точка відновлення…");
            ensure_restore_point(&app, allow_without_restore_point).map_err(abort)?;
        }
        let (cancel, rules) = {
            let mut c = lock(&state.cleanup);
            if let Some(why) = c.busy() {
                return Err(abort(failed(why)));
            }
            // План виконується один раз.
            c.plan = None;
            let t = CancelToken::new();
            c.exec = Some(t.clone());
            (t, c.rules())
        };
        let settings = lock(&state.settings).cleanup.clone();
        let handle = app.clone();
        let spawned = std::thread::Builder::new()
            .name("optimax-cleanup-exec".into())
            .spawn(move || {
                let result = run_clean(&handle, jid, &rules, &plan, &settings, &cancel);
                // Спершу знімаємо «зайнято», потім повідомляємо UI: він одразу запускає новий скан.
                lock(&handle.state::<AppState>().cleanup).exec = None;
                match result {
                    Ok(view) => {
                        let _ = handle.emit("cleanup-exec-done", view);
                    }
                    Err(e) => {
                        tracing::error!("очищення не вдалося: {e:?}");
                        let _ = handle.emit("cleanup-error", e);
                    }
                }
            });
        if let Err(e) = spawned {
            lock(&state.cleanup).exec = None;
            return Err(abort(failed(e)));
        }
        Ok(())
    })
    .await
}

fn run_clean(
    app: &AppHandle,
    jid: u64,
    rules: &RuleSet,
    plan: &CleanPlan,
    settings: &CleanupSettings,
    cancel: &CancelToken,
) -> CmdResult<ExecView> {
    let fs = StdFs;
    let journal = CleanupJournal::new(&fs, &journal_path()?);
    let needs_q = plan.items.iter().any(|i| i.action == Action::Quarantine);
    let mut quarantine = if needs_q {
        Some(Quarantine::open(
            &fs,
            &quarantine_index()?,
            &quarantine_root(settings)?,
            settings.quarantine_days,
        )?)
    } else {
        None
    };
    let ctx = ExecContext {
        fs: &fs,
        rules,
        services: Some(&SERVICES),
        now_unix: now_unix(),
        cancel,
    };
    let result = execute(&ctx, plan, quarantine.as_mut(), &|p| {
        let _ = app.emit("cleanup-exec-progress", p);
    });
    let res = match result {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            let _ = journal.finish(jid, now_unix(), Vec::new(), |r| r.note = Some(msg));
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
    let record = journal.finish(jid, now_unix(), res.lines.clone(), |r| {
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

/// Зупиняє очищення після поточного файлу.
#[tauri::command]
pub fn cleanup_exec_cancel(state: State<'_, AppState>) {
    if let Some(c) = lock(&state.cleanup).exec.as_ref() {
        c.cancel();
    }
}

/// Журнал очищення (найновіші — останні).
#[tauri::command]
pub async fn cleanup_journal() -> CmdResult<Vec<JournalRecord>> {
    blocking(|| Ok(CleanupJournal::new(&StdFs, &journal_path()?).load()?)).await
}

/// «Що виросло»: останній знімок диска порівняно зі знімком приблизно тижневої давності.
#[tauri::command]
pub async fn cleanup_growth(app: AppHandle, drive: String) -> CmdResult<Option<GrowthView>> {
    blocking(move || {
        let letter = path::normalize(&drive)
            .ok()
            .filter(|r| path::is_drive_root(r))
            .and_then(|r| path::drive_letter(&r))
            .ok_or_else(|| CmdError::Forbidden(format!("«{drive}» — не корінь диска")))?;
        let all = SnapshotStore::new(&StdFs, &data_dir()?, letter).load()?;
        let Some((current, history)) = all.split_last() else {
            return Ok(None);
        };
        let Some(base) = pick_baseline(history, current.unix, GROWTH_WINDOW_SECS) else {
            return Ok(None);
        };
        let rules = lock(&app.state::<AppState>().cleanup).rules();
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
    })
    .await
}

fn quarantine_view(settings: &CleanupSettings) -> CmdResult<QuarantineView> {
    let fs = StdFs;
    let dir = quarantine_root(settings)?;
    let q = Quarantine::open(&fs, &quarantine_index()?, &dir, settings.quarantine_days)?;
    let now = now_unix();
    let sys = system_drive();
    Ok(QuarantineView {
        on_system_drive: path::drive_letter(&dir) == path::drive_letter(&sys),
        batches: q
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
        other_drives: list_drives()
            .into_iter()
            .filter(|d| !d.is_system && !d.removable)
            .collect(),
        settings: settings.clone(),
        dir,
    })
}

/// Карантин: тека, налаштування, активні партії.
#[tauri::command]
pub async fn cleanup_quarantine(app: AppHandle) -> CmdResult<QuarantineView> {
    blocking(move || {
        let settings = lock(&app.state::<AppState>().settings).cleanup.clone();
        quarantine_view(&settings)
    })
    .await
}

/// Відновлює партію карантину.
#[tauri::command]
pub async fn cleanup_quarantine_restore(app: AppHandle, id: String) -> CmdResult<JournalRecord> {
    blocking(move || batch_action(&app, &id, RunKind::Restore)).await
}

/// Видаляє партію карантину остаточно.
#[tauri::command]
pub async fn cleanup_quarantine_purge(app: AppHandle, id: String) -> CmdResult<JournalRecord> {
    blocking(move || batch_action(&app, &id, RunKind::Purge)).await
}

fn batch_action(app: &AppHandle, id: &str, kind: RunKind) -> CmdResult<JournalRecord> {
    let _guard = ToolGuard::acquire(app)?;
    let state = app.state::<AppState>();
    let settings = lock(&state.settings).cleanup.clone();
    let rules = lock(&state.cleanup).rules();
    let fs = StdFs;
    let journal = CleanupJournal::new(&fs, &journal_path()?);
    let mut q = Quarantine::open(
        &fs,
        &quarantine_index()?,
        &quarantine_root(&settings)?,
        settings.quarantine_days,
    )?;
    let jid = journal.begin(kind, now_unix())?;
    let result = match kind {
        RunKind::Restore => q.restore(id, &rules),
        _ => q.purge(id),
    };
    let out = match result {
        Ok(o) => o,
        Err(e) => {
            let msg = e.to_string();
            let _ = journal.finish(jid, now_unix(), Vec::new(), |r| r.note = Some(msg));
            return Err(e.into());
        }
    };
    let record = journal.finish(jid, now_unix(), out.lines, |r| {
        r.batch_id = Some(id.to_string());
        match kind {
            RunKind::Restore => r.restored_bytes = out.bytes,
            _ => r.freed_bytes = out.bytes,
        }
    })?;
    tracing::info!(
        "карантин {id}: {} {} байтів",
        if kind == RunKind::Restore {
            "відновлено"
        } else {
            "видалено остаточно"
        },
        out.bytes
    );
    Ok(record)
}

/// Після старту: якщо користувач увімкнув автоочищення, видаляє партії з минулим терміном
/// (кожна — окремим записом у журналі).
pub fn startup_auto_purge(app: &AppHandle) {
    let settings = lock(&app.state::<AppState>().settings).cleanup.clone();
    if !settings.auto_purge {
        return;
    }
    let expired = quarantine_index()
        .and_then(|idx| {
            Quarantine::open(
                &StdFs,
                &idx,
                &quarantine_root(&settings)?,
                settings.quarantine_days,
            )
            .map_err(CmdError::from)
        })
        .map(|q| q.expired(now_unix()))
        .unwrap_or_default();
    for id in expired {
        match batch_action(app, &id, RunKind::Purge) {
            Ok(r) => tracing::info!(
                "автоочищення карантину: партія {id} видалена, звільнено {} байтів",
                r.freed_bytes
            ),
            Err(e) => tracing::warn!("автоочищення карантину: партія {id}: {e:?}"),
        }
    }
}

/// Штатне очищення компонентів Windows: `DISM /StartComponentCleanup`. Триває хвилини; звільнене
/// місце — виміряна різниця вільного місця системного диска.
#[tauri::command]
pub async fn cleanup_dism(
    app: AppHandle,
    allow_without_restore_point: bool,
) -> CmdResult<JournalRecord> {
    blocking(move || {
        let _guard = ToolGuard::acquire(&app)?;
        ensure_restore_point(&app, allow_without_restore_point)?;
        let sys = system_drive();
        let free = || drive_view(&sys).map(|d| d.free_bytes);
        let fs = StdFs;
        let journal = CleanupJournal::new(&fs, &journal_path()?);
        let jid = journal.begin(RunKind::Dism, now_unix())?;
        let before = free();
        tracing::info!("DISM /StartComponentCleanup: почато (може тривати кілька хвилин)");
        let out = WinTools.dism_component_cleanup();
        let after = free();
        let freed = match (before, after) {
            (Some(b), Some(a)) => a.saturating_sub(b),
            _ => 0,
        };
        let (ok, note) = match &out {
            Ok(o) => (o.ok(), format!("код завершення {}\n{}", o.code, o.output)),
            Err(e) => (false, e.to_string()),
        };
        let record = journal.finish(jid, now_unix(), Vec::new(), |r| {
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
    })
    .await
}

/// Вимикає (`enable = false`) або вмикає гібернацію через `powercfg`. Вимкнення — системна зміна:
/// перед нею точка відновлення; повернути — тією ж командою з `enable = true`.
#[tauri::command]
pub async fn cleanup_hibernate(
    app: AppHandle,
    enable: bool,
    allow_without_restore_point: bool,
) -> CmdResult<JournalRecord> {
    blocking(move || {
        let _guard = ToolGuard::acquire(&app)?;
        if !enable {
            ensure_restore_point(&app, allow_without_restore_point)?;
        }
        let fs = StdFs;
        let hiberfil = path::join(&system_drive(), "hiberfil.sys");
        let size = || fs.metadata(&hiberfil).ok().map(|m| m.len);
        let before = size();
        let journal = CleanupJournal::new(&fs, &journal_path()?);
        let kind = if enable {
            RunKind::HibernateOn
        } else {
            RunKind::HibernateOff
        };
        let jid = journal.begin(kind, now_unix())?;
        let out = WinTools.set_hibernate(enable);
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
        let record = journal.finish(jid, now_unix(), Vec::new(), |r| {
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
    })
    .await
}

/// Показує об'єкт у Провіднику. Лише шляхи з останнього скану або тека карантину.
#[tauri::command]
pub fn cleanup_reveal(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = path::normalize(&path).map_err(|e| CmdError::Forbidden(e.to_string()))?;
    let known = lock(&state.cleanup).report.as_ref().is_some_and(|r| {
        r.items
            .iter()
            .chain(r.unknown.iter())
            .chain(r.protected_zones.iter())
            .any(|i| {
                path::eq_ci(&i.path, &p) || i.top_subdirs.iter().any(|d| path::eq_ci(&d.path, &p))
            })
            || r.sizes.iter().any(|(s, _)| path::eq_ci(s, &p))
    });
    let settings = lock(&state.settings).cleanup.clone();
    let in_quarantine = quarantine_root(&settings).is_ok_and(|q| path::is_within(&p, &q));
    if !known && !in_quarantine {
        return Err(CmdError::Forbidden(format!("{p}: невідомий шлях")));
    }
    WinTools.reveal(&p)?;
    Ok(())
}
