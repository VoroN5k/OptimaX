//! Очищення диска: скан (`cleanup-progress`, `cleanup-scan-partial`, `cleanup-scan-done`,
//! `cleanup-error`), сухий прогін,
//! очищення (`cleanup-exec-progress`, `cleanup-exec-done`), карантин, журнал, «Що виросло» і
//! штатні засоби Windows (DISM, powercfg). Вердикти визначає лише бекенд.
//!
//! Логіка — [`optimax_backend::cleanup::Cleaner`]: у службі OptimaX, якщо вона доступна (етап 3в
//! з `docs/SERVICE.md`), інакше в цьому процесі. Тут — вибір шляху, події для UI й те, що
//! потребує сесії користувача: список дисків і «Показати в Провіднику».

use super::blocking;
use crate::error::{CmdError, CmdResult};
use crate::settings::CleanupSettings;
use crate::state::{lock, AppState};
use optimax_backend::cleanup::{
    BatchView, CleanResult, Cleaner, CleanupEvents, DryRunView, GrowthView, LastScan,
    QuarantineSpec, QuarantineState, ScanDone, ScanMode, WinPlatform,
};
use optimax_cleanup::journal::JournalRecord;
use optimax_cleanup::path;
use optimax_cleanup::scan::ScanReport;
use optimax_cleanup::system::{SystemTools, WinTools};
use optimax_service::client::Client;
use optimax_service::protocol::Command;
use optimax_service::ServiceError;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Як часто опитувати службу під час скану й очищення.
const POLL: Duration = Duration::from_millis(150);

/// Стан модуля очищення в програмі.
pub struct CleanupState {
    /// Очищення в цьому процесі (без служби); створюється, коли знадобиться.
    local: Option<Arc<Cleaner>>,
    /// Шляхи, показані на екрані (скан, «Що виросло»): лише їх можна відкрити в Провіднику.
    seen: HashSet<String>,
    /// Теки карантину, показані на екрані.
    quarantine_dirs: Vec<String>,
    /// Номер цього запуску програми (служба створює нову точку відновлення для нової сесії).
    session: u64,
}

impl Default for CleanupState {
    fn default() -> Self {
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self {
            local: None,
            seen: HashSet::new(),
            quarantine_dirs: Vec::new(),
            session: (u64::from(std::process::id()) << 32) | (started & 0xFFFF_FFFF),
        }
    }
}

impl CleanupState {
    fn local(&mut self) -> Arc<Cleaner> {
        self.local
            .get_or_insert_with(|| Arc::new(Cleaner::new(Box::new(WinPlatform::local()))))
            .clone()
    }

    fn remember(&mut self, report: &ScanReport) {
        for i in report
            .items
            .iter()
            .chain(report.unknown.iter())
            .chain(report.protected_zones.iter())
        {
            self.seen.insert(i.path.to_lowercase());
            for d in &i.top_subdirs {
                self.seen.insert(d.path.to_lowercase());
            }
        }
        for (p, _) in &report.sizes {
            self.seen.insert(p.to_lowercase());
        }
    }

    fn remember_quarantine(&mut self, dir: &str) {
        if !self.quarantine_dirs.iter().any(|d| path::eq_ci(d, dir)) {
            self.quarantine_dirs.push(dir.to_string());
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

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

/// Захищена тека даних (`%ProgramData%\OptimaX\<SID>`) — для читання звітом. Без прав
/// адміністратора її не створюємо й не перевіряємо (це робить служба), лише читаємо.
pub(crate) fn data_dir() -> CmdResult<String> {
    let dir = if optimax_core::process::is_elevated() {
        optimax_core::datadir::data_dir()
            .map_err(failed)?
            .to_path_buf()
    } else {
        optimax_core::datadir::protected_path()
            .ok_or_else(|| failed("не знайдено %ProgramData%"))?
    };
    path::normalize(&dir.to_string_lossy()).map_err(failed)
}

pub(crate) fn journal_path() -> CmdResult<String> {
    Ok(path::join(&data_dir()?, "cleanup-journal.json"))
}

fn system_drive() -> String {
    optimax_core::sysdirs::system_drive()
        .and_then(|d| path::normalize(&format!("{d}\\")).ok())
        .unwrap_or_else(|| r"C:\".to_string())
}

fn spec(state: &AppState) -> QuarantineSpec {
    let s = lock(&state.settings).cleanup.clone();
    QuarantineSpec {
        drive: s.quarantine_drive,
        days: s.quarantine_days,
    }
}

fn session(state: &AppState) -> u64 {
    lock(&state.cleanup).session
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

fn make_view(scan: LastScan) -> ScanView {
    let LastScan {
        report,
        rule_issues,
    } = scan;
    let sum = |v: optimax_cleanup::Verdict| {
        report
            .items
            .iter()
            .filter(|i| i.selectable && i.verdict == v)
            .map(|i| i.eligible_bytes)
            .sum::<u64>()
    };
    let (safe_bytes, consequences_bytes) = (
        sum(optimax_cleanup::Verdict::Safe),
        sum(optimax_cleanup::Verdict::Consequences),
    );
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

// ---- Куди йти: служба чи цей процес ----

enum Route {
    Local(Arc<Cleaner>),
    Remote(Client),
}

/// Служба, якщо вона доступна (і з'єднання вдалося), інакше — очищення в цьому процесі.
fn route(state: &AppState) -> Route {
    if state.link.service_usable() {
        match Client::connect() {
            Ok(c) => return Route::Remote(c),
            Err(e) => state.link.forget_service(&e),
        }
    }
    Route::Local(lock(&state.cleanup).local())
}

/// Команда службі: відповідь — результат її [`Cleaner`]. Обрив — помилка (стан дії невідомий),
/// наступна дія піде вже без служби.
fn ask<T: serde::de::DeserializeOwned>(
    c: &mut Client,
    state: &AppState,
    cmd: Command,
) -> CmdResult<T> {
    match c.json::<CleanResult<T>>(cmd) {
        Ok(r) => r.map_err(CmdError::from),
        Err(ServiceError::Refused(m)) => Err(CmdError::Failed(m)),
        Err(e) => {
            state.link.forget_service(&e);
            Err(CmdError::Failed(format!(
                "служба OptimaX не відповіла: {e}"
            )))
        }
    }
}

/// Дія через службу або в цьому процесі — та сама логіка.
fn call<T: serde::de::DeserializeOwned>(
    state: &AppState,
    cmd: Command,
    local: impl FnOnce(&Cleaner) -> CleanResult<T>,
) -> CmdResult<T> {
    match route(state) {
        Route::Remote(mut c) => ask(&mut c, state, cmd),
        Route::Local(c) => local(&c).map_err(CmdError::from),
    }
}

/// Чого чекати, опитуючи службу.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    Scan,
    Exec,
}

/// Опитує службу до кінця скану чи очищення й передає хід в інтерфейс тими самими подіями, що й
/// без служби. Повертає завершений скан (для [`Wait::Scan`]).
fn remote_wait(app: &AppHandle, mut c: Client, wait: Wait) -> CmdResult<Option<ScanDone>> {
    let state = app.state::<AppState>();
    loop {
        std::thread::sleep(POLL);
        let e: CleanupEvents = ask(&mut c, &state, Command::CleanupPoll)?;
        if let Some(p) = e.scan_progress {
            let _ = app.emit("cleanup-progress", p);
        }
        if let Some(s) = e.scan_partial {
            let _ = app.emit("cleanup-scan-partial", make_view(s));
        }
        if let Some(p) = e.exec_progress {
            let _ = app.emit("cleanup-exec-progress", p);
        }
        if let Some(d) = e.scan_done {
            lock(&state.cleanup).remember(&d.scan.report);
            if d.kept {
                let _ = app.emit("cleanup-scan-done", make_view(d.scan.clone()));
            }
            if wait == Wait::Scan {
                return Ok(Some(d));
            }
        }
        if let Some(r) = e.exec_done {
            match r {
                Ok(v) => {
                    let _ = app.emit("cleanup-exec-done", v);
                }
                Err(e) => {
                    let _ = app.emit("cleanup-error", CmdError::from(e));
                }
            }
            if wait == Wait::Exec {
                return Ok(None);
            }
        }
    }
}

/// Опитування служби в окремому потоці (скан чи очищення екрана «Очищення»).
fn spawn_wait(app: &AppHandle, c: Client, wait: Wait) -> CmdResult<()> {
    let handle = app.clone();
    std::thread::Builder::new()
        .name("optimax-cleanup-poll".into())
        .spawn(move || {
            if let Err(e) = remote_wait(&handle, c, wait) {
                tracing::warn!("очищення через службу: {e:?}");
                let _ = handle.emit("cleanup-error", e);
            }
        })
        .map(|_| ())
        .map_err(failed)
}

/// При виході: зупиняє скан і очищення після поточного файлу (DISM не переривається) — і в
/// цьому процесі, і в службі.
pub fn cancel_all(state: &AppState) {
    if let Some(c) = lock(&state.cleanup).local.clone() {
        c.cancel_all();
    }
    if let Route::Remote(mut c) = route(state) {
        let _ = c.json::<CleanResult<()>>(Command::CleanupScanCancel);
        let _ = c.json::<CleanResult<()>>(Command::CleanupExecCancel);
    }
}

// ---- Команди ----

/// Диски.
#[tauri::command]
pub async fn cleanup_drives() -> CmdResult<Vec<DriveView>> {
    blocking(|| Ok(list_drives())).await
}

fn last_scan(state: &AppState) -> CmdResult<Option<LastScan>> {
    let scan = call(state, Command::CleanupLastScan, |c| Ok(c.last_scan()))?;
    if let Some(s) = &scan {
        lock(&state.cleanup).remember(&s.report);
    }
    Ok(scan)
}

/// Останній звіт скану цієї сесії (щоб екран не сканував заново після переходу між вкладками).
#[tauri::command]
pub async fn cleanup_last_scan(app: AppHandle) -> CmdResult<Option<ScanView>> {
    blocking(move || Ok(last_scan(&app.state::<AppState>())?.map(make_view))).await
}

/// Сканує в цьому процесі: хід і результат — подіями (проміжні — лише для звіту екрана).
fn run_local_scan(
    app: &AppHandle,
    c: &Cleaner,
    job: optimax_backend::cleanup::ScanJob,
) -> ScanDone {
    let kept = job.keeps();
    let report = job.run(
        c,
        &|p| {
            let _ = app.emit("cleanup-progress", p);
        },
        &|r| {
            let s = LastScan {
                report: r.clone(),
                rule_issues: c.rule_issues(),
            };
            let _ = app.emit("cleanup-scan-partial", make_view(s));
        },
    );
    lock(&app.state::<AppState>().cleanup).remember(&report);
    let scan = LastScan {
        report,
        rule_issues: c.rule_issues(),
    };
    if kept {
        let _ = app.emit("cleanup-scan-done", make_view(scan.clone()));
    }
    ScanDone { scan, kept }
}

/// Починає скан диска. `full` — обхід усього диска (довше; показує «Невідоме» й дає повний знімок).
#[tauri::command]
pub async fn cleanup_scan_start(app: AppHandle, drive: String, full: bool) -> CmdResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        match route(&state) {
            Route::Remote(mut c) => {
                let cmd = Command::CleanupScanStart {
                    drive,
                    full,
                    background: false,
                };
                ask::<()>(&mut c, &state, cmd)?;
                spawn_wait(&app, c, Wait::Scan)
            }
            Route::Local(c) => {
                let job = c.start_scan(&drive, full, ScanMode::Screen)?;
                let handle = app.clone();
                let cleaner = c.clone();
                let slot = Arc::new(std::sync::Mutex::new(Some(job)));
                let in_thread = slot.clone();
                let spawned = std::thread::Builder::new()
                    .name("optimax-cleanup-scan".into())
                    .spawn(move || {
                        let job = in_thread
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        if let Some(job) = job {
                            run_local_scan(&handle, &cleaner, job);
                        }
                    });
                if let Err(e) = spawned {
                    let job = slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    if let Some(job) = job {
                        job.abandon(&c);
                    }
                    return Err(failed(e));
                }
                Ok(())
            }
        }
    })
    .await
}

/// Швидкий скан системного диска за правилами для головного екрана (блокує потік; лише
/// метадані, нічого не видаляє). Звіт стає й звітом екрана «Очищення», якщо там ще порожньо.
pub(crate) fn scan_system_drive(app: &AppHandle) -> CmdResult<ScanView> {
    let state = app.state::<AppState>();
    let root = system_drive();
    let done = match route(&state) {
        Route::Remote(mut c) => {
            let cmd = Command::CleanupScanStart {
                drive: root,
                full: false,
                background: true,
            };
            ask::<()>(&mut c, &state, cmd)?;
            remote_wait(app, c, Wait::Scan)?
                .ok_or_else(|| failed("служба не повернула звіт скану"))?
        }
        Route::Local(c) => {
            let job = c.start_scan(&root, false, ScanMode::Background)?;
            run_local_scan(app, &c, job)
        }
    };
    Ok(make_view(done.scan))
}

/// Останній завершений скан системного диска цієї сесії (з екрана «Очищення» чи головного).
pub(crate) fn last_system_scan(state: &AppState) -> Option<ScanView> {
    let root = system_drive();
    last_scan(state)
        .ok()
        .flatten()
        .filter(|s| !s.report.cancelled && path::eq_ci(&s.report.drive, &root))
        .map(make_view)
}

/// Зупиняє скан (звіт — за тим, що встигли побачити).
#[tauri::command]
pub async fn cleanup_scan_cancel(app: AppHandle) -> CmdResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        // Скасування не падає: служба зникла — то й дії вже немає.
        if let Err(e) = call(&state, Command::CleanupScanCancel, |c| {
            c.cancel_scan();
            Ok(())
        }) {
            tracing::warn!("скасування очищення: {e:?}");
        }
        Ok(())
    })
    .await
}

/// Сухий прогін: точний список файлів, сума й вердикти. Нічого не змінює.
#[tauri::command]
pub async fn cleanup_dry_run(app: AppHandle, paths: Vec<String>) -> CmdResult<DryRunView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let q = spec(&state);
        let cmd = Command::CleanupDryRun {
            paths: paths.clone(),
            quarantine_drive: q.drive.clone(),
            quarantine_days: q.days,
        };
        let view = call(&state, cmd, |c| c.dry_run(&paths, &q))?;
        lock(&state.cleanup).remember_quarantine(&view.quarantine_dir);
        Ok(view)
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
        let q = spec(&state);
        let session = session(&state);
        match route(&state) {
            Route::Remote(mut c) => {
                let cmd = Command::CleanupExecute {
                    plan_id,
                    allow_without_restore_point,
                    session,
                    quarantine_drive: q.drive,
                    quarantine_days: q.days,
                };
                ask::<()>(&mut c, &state, cmd)?;
                spawn_wait(&app, c, Wait::Exec)
            }
            Route::Local(c) => {
                let job = c.start_exec(plan_id, allow_without_restore_point, session, &q)?;
                let handle = app.clone();
                let cleaner = c.clone();
                let slot = Arc::new(std::sync::Mutex::new(Some(job)));
                let in_thread = slot.clone();
                let spawned = std::thread::Builder::new()
                    .name("optimax-cleanup-exec".into())
                    .spawn(move || {
                        let job = in_thread
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        let Some(job) = job else { return };
                        // Модуль звільняється до повідомлення: UI одразу запускає новий скан.
                        match job.run(&cleaner, &|p| {
                            let _ = handle.emit("cleanup-exec-progress", p);
                        }) {
                            Ok(view) => {
                                let _ = handle.emit("cleanup-exec-done", view);
                            }
                            Err(e) => {
                                let _ = handle.emit("cleanup-error", CmdError::from(e));
                            }
                        }
                    });
                if let Err(e) = spawned {
                    let job = slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    if let Some(job) = job {
                        job.abandon(&c, &e.to_string());
                    }
                    return Err(failed(e));
                }
                Ok(())
            }
        }
    })
    .await
}

/// Зупиняє очищення після поточного файлу.
#[tauri::command]
pub async fn cleanup_exec_cancel(app: AppHandle) -> CmdResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        // Скасування не падає: служба зникла — то й дії вже немає.
        if let Err(e) = call(&state, Command::CleanupExecCancel, |c| {
            c.cancel_exec();
            Ok(())
        }) {
            tracing::warn!("скасування очищення: {e:?}");
        }
        Ok(())
    })
    .await
}

/// Журнал очищення (найновіші — останні).
#[tauri::command]
pub async fn cleanup_journal(app: AppHandle) -> CmdResult<Vec<JournalRecord>> {
    blocking(move || {
        call(
            &app.state::<AppState>(),
            Command::CleanupJournal,
            Cleaner::journal_records,
        )
    })
    .await
}

/// «Що виросло»: останній знімок диска порівняно зі знімком приблизно тижневої давності.
#[tauri::command]
pub async fn cleanup_growth(app: AppHandle, drive: String) -> CmdResult<Option<GrowthView>> {
    blocking(move || {
        let state = app.state::<AppState>();
        let cmd = Command::CleanupGrowth {
            drive: drive.clone(),
        };
        let view = call(&state, cmd, |c| c.growth(&drive))?;
        if let Some(v) = &view {
            let mut st = lock(&state.cleanup);
            for i in &v.items {
                st.seen.insert(i.growth.path.to_lowercase());
            }
        }
        Ok(view)
    })
    .await
}

/// Карантин: тека, налаштування, активні партії.
#[tauri::command]
pub async fn cleanup_quarantine(app: AppHandle) -> CmdResult<QuarantineView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let settings = lock(&state.settings).cleanup.clone();
        let q = spec(&state);
        let cmd = Command::CleanupQuarantine {
            quarantine_drive: q.drive.clone(),
            quarantine_days: q.days,
        };
        let QuarantineState {
            dir,
            on_system_drive,
            batches,
        } = call(&state, cmd, |c| c.quarantine(&q))?;
        {
            let mut st = lock(&state.cleanup);
            st.remember_quarantine(&dir);
            for b in &batches {
                st.remember_quarantine(&b.dir);
            }
        }
        Ok(QuarantineView {
            dir,
            settings,
            on_system_drive,
            batches,
            other_drives: list_drives()
                .into_iter()
                .filter(|d| !d.is_system && !d.removable)
                .collect(),
        })
    })
    .await
}

fn batch_action(state: &AppState, id: &str, restore: bool) -> CmdResult<JournalRecord> {
    let q = spec(state);
    let cmd = Command::CleanupBatch {
        id: id.to_string(),
        restore,
        quarantine_drive: q.drive.clone(),
        quarantine_days: q.days,
    };
    call(state, cmd, |c| c.batch(id, restore, &q))
}

/// Відновлює партію карантину.
#[tauri::command]
pub async fn cleanup_quarantine_restore(app: AppHandle, id: String) -> CmdResult<JournalRecord> {
    blocking(move || batch_action(&app.state::<AppState>(), &id, true)).await
}

/// Видаляє партію карантину остаточно.
#[tauri::command]
pub async fn cleanup_quarantine_purge(app: AppHandle, id: String) -> CmdResult<JournalRecord> {
    blocking(move || batch_action(&app.state::<AppState>(), &id, false)).await
}

/// Після старту: якщо користувач увімкнув автоочищення, видаляє партії з минулим терміном
/// (кожна — окремим записом у журналі).
pub fn startup_auto_purge(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !lock(&state.settings).cleanup.auto_purge {
        return;
    }
    let q = spec(&state);
    let cmd = Command::CleanupAutoPurge {
        quarantine_drive: q.drive.clone(),
        quarantine_days: q.days,
    };
    match call(&state, cmd, |c| Ok(c.auto_purge(&q))) {
        Ok(0) => {}
        Ok(n) => tracing::info!("автоочищення карантину: видалено партій — {n}"),
        Err(e) => tracing::warn!("автоочищення карантину: {e:?}"),
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
        let state = app.state::<AppState>();
        let session = session(&state);
        let cmd = Command::CleanupDism {
            allow_without_restore_point,
            session,
        };
        call(&state, cmd, |c| {
            c.dism(allow_without_restore_point, session)
        })
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
        let state = app.state::<AppState>();
        let session = session(&state);
        let cmd = Command::CleanupHibernate {
            enable,
            allow_without_restore_point,
            session,
        };
        call(&state, cmd, |c| {
            c.hibernate(enable, allow_without_restore_point, session)
        })
    })
    .await
}

/// Показує об'єкт у Провіднику (у сесії користувача, тому завжди з програми). Лише шляхи, які
/// екран отримав від скану чи «Що виросло», або тека карантину.
#[tauri::command]
pub fn cleanup_reveal(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = path::normalize(&path).map_err(|e| CmdError::Forbidden(e.to_string()))?;
    let allowed = {
        let st = lock(&state.cleanup);
        st.seen.contains(&p.to_lowercase())
            || st.quarantine_dirs.iter().any(|q| path::is_within(&p, q))
    };
    if !allowed {
        return Err(CmdError::Forbidden(format!("{p}: невідомий шлях")));
    }
    WinTools.reveal(&p)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_allows_only_shown_paths() {
        let mut st = CleanupState::default();
        let report = ScanReport {
            drive: r"C:\".into(),
            full: false,
            started_unix: 0,
            duration_ms: 0,
            items: Vec::new(),
            unknown: Vec::new(),
            protected_zones: Vec::new(),
            scanned_bytes: 0,
            unreadable_dirs: 0,
            cancelled: false,
            sizes: vec![(r"C:\Users\Test\AppData\Local\Temp".into(), 10)],
        };
        st.remember(&report);
        st.remember_quarantine(r"D:\OptimaX\S-1-5-21-1\Quarantine");
        assert!(st.seen.contains(r"c:\users\test\appdata\local\temp"));
        assert!(!st.seen.contains(r"c:\windows"));
        st.remember_quarantine(r"d:\optimax\s-1-5-21-1\quarantine");
        assert_eq!(st.quarantine_dirs.len(), 1);
    }
}
