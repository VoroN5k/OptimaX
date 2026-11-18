//! Бібліотека ігор: список встановлених ігор, обкладинки, приховування, визначення процесу гри
//! за текою встановлення. Лише читання файлів лаунчерів і реєстру; стан людини — `library.json`.

use super::blocking;
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_frametime::Target;
use optimax_gamemode::library::detect::{
    after_recording, pick_game_in, strip_verbatim, AfterRecording, DetectState, Detector,
    RecordingOutcome, RunningProcess,
};
use optimax_gamemode::library::fs::{LibraryFs, StdFs};
use optimax_gamemode::library::scan::{self, Sources};
use optimax_gamemode::library::store::LibraryStore;
use optimax_gamemode::{GameEntry, LibraryReport};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// Файл стану бібліотеки в теці даних інтерфейсу.
const STORE_FILE: &str = "library.json";
/// Найбільша обкладинка, яку віддаємо в UI (файли Steam — десятки КБ).
const MAX_COVER_BYTES: u64 = 2 * 1024 * 1024;

/// Кеш бібліотеки й поточне визначення процесу.
#[derive(Default)]
pub struct LibraryState {
    report: Option<LibraryReport>,
    detect: Option<Detecting>,
    /// Визначений, але ще не підтверджений записом exe: `(id гри, exe)`.
    pending: Option<(String, String)>,
    /// Exe, що закрились посеред запису (схоже на лаунчер), — пропускаються до кінця сесії.
    skipped: HashMap<String, Vec<String>>,
}

/// Поточне визначення: фоновий потік і як його зупинити.
struct Detecting {
    /// Прапорець зупинки фонового потоку.
    cancel: Arc<AtomicBool>,
}

fn store_path() -> CmdResult<PathBuf> {
    let dir = optimax_core::datadir::ui_dir().map_err(|e| CmdError::Failed(e.to_string()))?;
    Ok(dir.join(STORE_FILE))
}

fn sources() -> Sources {
    Sources {
        steam_root: scan::steam_root(&optimax_core::registry::WinRegistry),
        epic_manifests: scan::default_epic_manifests(),
    }
}

fn rescan(state: &AppState) -> CmdResult<LibraryReport> {
    let store = LibraryStore::load(&store_path()?);
    let report = scan::scan_library(
        &StdFs,
        &optimax_core::registry::WinRegistry,
        &sources(),
        &store,
    );
    for p in &report.problems {
        tracing::warn!(
            "бібліотека ігор: {:?} {} — {}",
            p.launcher,
            p.what,
            p.reason
        );
    }
    lock(&state.library).report = Some(report.clone());
    Ok(report)
}

/// Змінює стан людини й оновлює кеш (без повторного скану дисків).
fn update_store(state: &AppState, f: impl FnOnce(&mut LibraryStore)) -> CmdResult<()> {
    let path = store_path()?;
    let mut store = LibraryStore::load(&path);
    f(&mut store);
    store
        .save(&path)
        .map_err(|e| CmdError::Failed(e.to_string()))?;
    let mut lib = lock(&state.library);
    if let Some(report) = lib.report.as_mut() {
        for g in &mut report.games {
            if let Some(u) = store.games.get(&g.id) {
                if g.launcher != optimax_gamemode::Launcher::Manual {
                    g.exe.clone_from(&u.exe);
                }
                g.hidden = u.hidden;
            }
        }
    }
    Ok(())
}

fn find_game(state: &AppState, id: &str) -> CmdResult<GameEntry> {
    let cached = lock(&state.library)
        .report
        .as_ref()
        .and_then(|r| r.games.iter().find(|g| g.id == id).cloned());
    match cached {
        Some(g) => Ok(g),
        None => rescan(state)?
            .games
            .into_iter()
            .find(|g| g.id == id)
            .ok_or_else(|| CmdError::Failed(format!("гру {id} не знайдено в бібліотеці"))),
    }
}

/// Список ігор. `refresh` або порожній кеш — новий скан (читання кількох файлів і ключів).
#[tauri::command]
pub async fn library_scan(app: AppHandle, refresh: bool) -> CmdResult<LibraryReport> {
    blocking(move || {
        let state = app.state::<AppState>();
        if !refresh {
            if let Some(r) = lock(&state.library).report.clone() {
                return Ok(r);
            }
        }
        rescan(&state)
    })
    .await
}

/// Приховує або показує гру.
#[tauri::command]
pub async fn library_set_hidden(app: AppHandle, id: String, hidden: bool) -> CmdResult<()> {
    blocking(move || update_store(&app.state::<AppState>(), |s| s.set_hidden(&id, hidden))).await
}

/// Забуває визначений exe гри (наступний запис визначить його знову).
#[tauri::command]
pub async fn library_forget_exe(app: AppHandle, id: String) -> CmdResult<()> {
    blocking(move || {
        tracing::info!("бібліотека ігор: процес гри {id} скинуто");
        let state = app.state::<AppState>();
        lock(&state.library).skipped.remove(&id);
        update_store(&state, |s| s.set_exe(&id, None))
    })
    .await
}

/// Обкладинка гри як `data:`-адреса (лише локальні файли Steam); `None` — обкладинки немає.
#[tauri::command]
pub async fn library_cover(id: String) -> CmdResult<Option<String>> {
    blocking(move || {
        let Some(path) = scan::find_cover(&StdFs, &sources(), &id) else {
            return Ok(None);
        };
        Ok(StdFs
            .read_bytes(&path, MAX_COVER_BYTES)
            .ok()
            .map(|b| format!("data:image/jpeg;base64,{}", crate::b64::encode(&b))))
    })
    .await
}

/// Додає гру вручну за ім'ям exe.
#[tauri::command]
pub async fn library_add_manual(app: AppHandle, name: String, exe: String) -> CmdResult<GameEntry> {
    blocking(move || {
        let exe = exe.trim().to_string();
        Target::ProcessName(exe.clone()).validate()?;
        let name = if name.trim().is_empty() {
            exe.clone()
        } else {
            name.trim().to_string()
        };
        let state = app.state::<AppState>();
        let path = store_path()?;
        let mut store = LibraryStore::load(&path);
        let id = store.add_manual(&name, &exe);
        store
            .save(&path)
            .map_err(|e| CmdError::Failed(e.to_string()))?;
        tracing::info!("бібліотека ігор: додано вручну {name} ({exe})");
        rescan(&state)?
            .games
            .into_iter()
            .find(|g| g.id == id)
            .ok_or_else(|| CmdError::Failed("гру не додано".into()))
    })
    .await
}

/// Як часто дивитись на передній план, поки людина запускає гру.
const DETECT_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// Починає визначення процесу гри у фоновому потоці (як запис: опитування з Rust, а не з
/// інтерфейсу — поки гра на весь екран, WebView2 може пригальмовувати свої таймери).
/// Події `library-detect`: `waiting` (на старті й коли змінився список пропущених), `found`.
/// Гра — процес з теки гри на передньому плані, найважчий серед процесів цієї теки; вибір має
/// повторитися кілька разів поспіль. Збережеться exe лише після запису до кінця ([`after_record`]).
#[tauri::command]
pub async fn library_detect_start(app: AppHandle, id: String) -> CmdResult<DetectState> {
    blocking(move || {
        let state = app.state::<AppState>();
        let game = find_game(&state, &id)?;
        if game.install_dir.is_empty() {
            return Ok(DetectState::NoFolder);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let skipped = {
            let mut lib = lock(&state.library);
            if let Some(old) = lib.detect.replace(Detecting {
                cancel: cancel.clone(),
            }) {
                old.cancel.store(true, Ordering::Relaxed);
            }
            lib.skipped.get(&id).cloned().unwrap_or_default()
        };
        let thread_app = app.clone();
        std::thread::Builder::new()
            .name("optimax-library-detect".into())
            .spawn(move || detect_loop(&thread_app, &game, &cancel))
            .map_err(|e| CmdError::Failed(format!("потік визначення гри не запустився: {e}")))?;
        Ok(DetectState::Waiting { skipped })
    })
    .await
}

fn detect_loop(app: &AppHandle, game: &GameEntry, cancel: &AtomicBool) {
    let state = app.state::<AppState>();
    // Гру, перенесену через junction, Windows запускає з реального шляху.
    let canonical = std::fs::canonicalize(&game.install_dir)
        .ok()
        .map(|p| strip_verbatim(&p.to_string_lossy()));
    let mut dirs = vec![game.install_dir.as_str()];
    dirs.extend(canonical.as_deref());
    let mut sys = sysinfo::System::new();
    let mut detector = Detector::default();
    let mut last_fg: Option<String> = None;
    let mut last_skipped: Vec<String> = Vec::new();
    tracing::info!(
        "бібліотека ігор: чекаю, поки «{}» з'явиться на екрані",
        game.name
    );
    while !cancel.load(Ordering::Relaxed) {
        let foreground = optimax_core::input::foreground_process_id();
        let procs = snapshot(&mut sys);
        // Що на передньому плані — у журнал дій лише при зміні: так видно, чому гру не впізнано.
        let fg = foreground.and_then(|pid| procs.iter().find(|p| p.pid == pid));
        let fg_line = fg.map(|p| match &p.path {
            Some(path) => format!("{} ({path})", p.name),
            None => format!("{} (шлях недоступний)", p.name),
        });
        if fg_line != last_fg {
            if let Some(line) = &fg_line {
                tracing::info!("бібліотека ігор: на передньому плані — {line}");
            }
            last_fg = fg_line;
        }
        let skipped = lock(&state.library)
            .skipped
            .get(&game.id)
            .cloned()
            .unwrap_or_default();
        if skipped != last_skipped {
            let _ = app.emit(
                "library-detect",
                DetectState::Waiting {
                    skipped: skipped.clone(),
                },
            );
            last_skipped = skipped.clone();
        }
        let picked = pick_game_in(&dirs, foreground, &procs, &skipped).map(|p| p.name.clone());
        if let Some(exe) = detector.feed(picked.as_deref()) {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            {
                let mut lib = lock(&state.library);
                lib.detect = None;
                lib.pending = Some((game.id.clone(), exe.clone()));
            }
            tracing::info!("бібліотека ігор: гру «{}» визначено — {exe}", game.name);
            let _ = app.emit("library-detect", DetectState::Found { exe });
            return;
        }
        std::thread::sleep(DETECT_POLL);
    }
}

/// Скасовує визначення процесу (людина передумала).
#[tauri::command]
pub async fn library_detect_cancel(app: AppHandle) -> CmdResult<()> {
    blocking(move || {
        if let Some(d) = lock(&app.state::<AppState>().library).detect.take() {
            d.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    })
    .await
}

/// Процеси з шляхом і пам'яттю (лише для визначення гри; CPU не рахується).
fn snapshot(sys: &mut sysinfo::System) -> Vec<RunningProcess> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    let kind = ProcessRefreshKind::nothing()
        .with_memory()
        .with_exe(UpdateKind::OnlyIfNotSet);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    sys.processes()
        .iter()
        .map(|(pid, p)| RunningProcess {
            pid: pid.as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            path: p.exe().map(|e| e.to_string_lossy().into_owned()),
            memory_mib: p.memory() / (1024 * 1024),
        })
        .collect()
}

/// Після запису: визначений exe зберігається, лише якщо запис дійшов до кінця з кадрами;
/// якщо процес закрився посеред запису — схоже на лаунчер, і до кінця сесії він пропускається.
pub fn after_record(state: &AppState, exe: &str, outcome: RecordingOutcome) {
    let pending = {
        let mut lib = lock(&state.library);
        match lib.pending.take() {
            Some((id, p)) if p.eq_ignore_ascii_case(exe) => Some(id),
            other => {
                lib.pending = other;
                None
            }
        }
    };
    let Some(id) = pending else {
        return;
    };
    match after_recording(outcome) {
        AfterRecording::Confirm => {
            if let Err(e) = update_store(state, |s| s.set_exe(&id, Some(exe.to_string()))) {
                tracing::warn!(
                    "бібліотека ігор: не вдалося зберегти процес гри: {}",
                    e.message()
                );
            }
        }
        AfterRecording::Skip => {
            tracing::info!(
                "бібліотека ігор: «{exe}» закрився посеред запису — схоже, це лаунчер; \
                 наступного разу програма чекатиме на саму гру"
            );
            lock(&state.library)
                .skipped
                .entry(id)
                .or_default()
                .push(exe.to_string());
        }
        AfterRecording::Nothing => {}
    }
}
