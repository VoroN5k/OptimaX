//! Лабораторія стабільності: запуск тесту окремим процесом, запобіжники, журнал подій і результати.
//!
//! Тест нічого не змінює в системі. Поки він іде, на диску лежить позначка (`running.json`): якщо
//! ПК вимкнеться чи програма зникне, наступний запуск покаже незавершений тест, а не загубить його.

use super::blocking;
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_audit::model::SystemEvent;
use optimax_audit::rules::{
    whea_kind, WheaSeverity, DISPLAY_PROVIDER, KERNEL_POWER_PROVIDER, TDR_EVENT_ID,
    UNEXPECTED_OFF_EVENT_ID, WHEA_PROVIDER,
};
use optimax_audit::{SystemProbe, WindowsProbe};
use optimax_frametime::CancelToken;
use optimax_history::{diff, StateChange, SystemState};
use optimax_metrics::{GpuApi, GpuSnapshot, NvmlGpu};
use optimax_stability::protocol::{worker_args, TestKind, WorkerMessage};
use optimax_stability::verdict::{
    duration_allowed, is_night, stop_on_error, Guard, GuardStop, StabEnd, StabEvents, StabInfo,
    StabRecord, StabView, StatsAcc, DURATIONS_S, GPU_STOP_TEMP_C, NIGHT_DURATIONS_S,
};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

/// Без вікна консолі в процесу навантаження.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Без жодного повідомлення так довго процес вважається завислим, с. Більше за запас на очікування
/// однієї порції GPU в самому процесі (30 с).
const HANG_S: f64 = 45.0;
/// Скільки чекати на завершення процесу після просьби зупинитись.
const STOP_WAIT: Duration = Duration::from_secs(5);
/// Журнал Windows записує подію не миттєво.
const EVENTS_SETTLE: Duration = Duration::from_secs(2);
/// Скільки збережених тестів тримати.
const KEEP_RECORDS: usize = 50;
/// Позначка тесту, що йде.
const MARKER: &str = "running.json";
const MIB: u64 = 1024 * 1024;

/// Сесія тесту.
pub struct StabSession {
    view: StabView,
    stop: CancelToken,
    state: Option<SystemState>,
}

/// Збережений результат разом зі станом ПК на початку тесту: так видно, на яких налаштуваннях
/// (BIOS, частота пам'яті, драйвер) тест пройшов чи впав.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct StabEntry {
    /// Результат.
    #[serde(flatten)]
    pub record: StabRecord,
    /// Стан ПК на початку тесту (`None` — старий запис або стан не прочитано).
    #[serde(default)]
    pub state: Option<SystemState>,
    /// Що змінилося від попереднього тесту того самого виду (на диск не пишеться).
    #[serde(default, skip_deserializing)]
    pub changes: Vec<StateChange>,
}

/// Позначка тесту, що йде: хід і стан ПК, щоб стан не загубився, якщо ПК вимкнеться.
#[derive(Serialize, Deserialize)]
struct Marker {
    #[serde(flatten)]
    view: StabView,
    #[serde(default)]
    state: Option<SystemState>,
}

/// Тест іде: інші записи гри тоді не запускаються.
pub fn is_active(state: &AppState) -> bool {
    lock(&state.stability).is_some()
}

/// Помилка «іде тест стабільності» для інших записів.
pub fn busy_error() -> CmdError {
    CmdError::Failed("іде тест стабільності — дочекайся кінця або зупини його".into())
}

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

fn dir() -> CmdResult<PathBuf> {
    let d = optimax_core::datadir::data_dir()
        .map_err(failed)?
        .join("stability");
    std::fs::create_dir_all(&d).map_err(failed)?;
    Ok(d)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Показання відеокарти, що працює найбільше (температура — найвища серед усіх).
fn read_sensors(gpu: &mut Option<NvmlGpu>) -> (Option<u32>, Option<GpuSnapshot>) {
    let Some(g) = gpu.as_mut() else {
        return (None, None);
    };
    let all = g.sample();
    let temp = all.iter().filter_map(|s| s.temperature_c).max();
    let busiest = all.into_iter().max_by_key(|s| s.utilization_percent);
    (temp, busiest)
}

/// Що відомо до запуску.
#[tauri::command]
pub async fn stab_info() -> CmdResult<StabInfo> {
    blocking(|| {
        let mut gpu = NvmlGpu::open();
        let (temp, busiest) = read_sensors(&mut gpu);
        let (ram_total, ram_free) = optimax_stability::ram::memory().unwrap_or((0, 0));
        Ok(StabInfo {
            gpu: busiest.map(|s| s.name),
            cpu: optimax_stability::cpu::brand(),
            cpu_cores: u32::try_from(optimax_stability::cpu::topology().len()).unwrap_or(0),
            ram_mb: ram_total / MIB,
            ram_test_mb: optimax_stability::ram::test_bytes(ram_total, ram_free) / MIB,
            has_sensor: temp.is_some(),
            limit_c: GPU_STOP_TEMP_C,
            durations_s: DURATIONS_S.to_vec(),
            night_durations_s: NIGHT_DURATIONS_S.to_vec(),
        })
    })
    .await
}

/// Запускає тест. Без датчика температури (CPU і RAM завжди) — лише з `accept_no_sensor`.
#[tauri::command]
pub async fn stab_start(
    app: AppHandle,
    kind: TestKind,
    seconds: u32,
    accept_no_sensor: bool,
) -> CmdResult<StabView> {
    blocking(move || start(&app, kind, seconds, accept_no_sensor)).await
}

fn start(
    app: &AppHandle,
    kind: TestKind,
    seconds: u32,
    accept_no_sensor: bool,
) -> CmdResult<StabView> {
    if !duration_allowed(seconds) {
        return Err(CmdError::Failed("недопустима тривалість тесту".into()));
    }
    let state = app.state::<AppState>();
    if lock(&state.recording).is_some()
        || lock(&state.ab).is_some()
        || super::resolution::is_running(&state)
        || super::experiment::is_active(&state)
        || super::lag::is_watching(&state)
    {
        return Err(CmdError::Failed(
            "іде запис гри або інший тест — спершу заверши його".into(),
        ));
    }
    if is_active(&state) {
        return Err(busy_error());
    }
    // Стан ПК — до запуску навантаження (WMI повільний, а під навантаженням ще повільніший).
    let pc = Some(super::history::system_state(""));
    let mut slot = lock(&state.stability);
    if slot.is_some() {
        return Err(busy_error());
    }
    let d = dir()?;
    // Незавершений попередній тест спершу стає результатом, щоб не загубити його позначку.
    recover(&d);
    // Датчиків CPU й RAM без драйвера ядра немає, тож і NVML для цих тестів не потрібен.
    let mut gpu = (kind == TestKind::Gpu).then(NvmlGpu::open).flatten();
    let (temp, _) = read_sensors(&mut gpu);
    let has_sensor = temp.is_some();
    if !has_sensor && !accept_no_sensor {
        return Err(CmdError::Failed(
            "температура недоступна — потрібне явне підтвердження".into(),
        ));
    }
    if temp.is_some_and(|t| t >= GPU_STOP_TEMP_C) {
        return Err(CmdError::Failed(format!(
            "GPU вже гарячий (≥ {GPU_STOP_TEMP_C} °C) — дай йому охолонути"
        )));
    }
    let exe = std::env::current_exe().map_err(failed)?;
    let mut child = Command::new(exe)
        .args(worker_args(kind, seconds))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| CmdError::Failed(format!("процес тесту не запустився: {e}")))?;
    let stdin = child.stdin.take();
    let messages = read_messages(&mut child)?;
    let started = now_ms();
    let view = StabView {
        id: format!("{started}-{}", kind.as_str()),
        kind,
        bad_cores: Vec::new(),
        current_core: None,
        total_cores: 0,
        started_unix_ms: started,
        adapter: String::new(),
        planned_s: seconds,
        elapsed_s: 0.0,
        rounds: 0,
        checked: 0,
        mismatches: 0,
        tested_mb: 0,
        vram_mb: 0,
        temp_c: temp,
        clock_mhz: None,
        power_w: None,
        util_pct: None,
        limit_c: has_sensor.then_some(GPU_STOP_TEMP_C),
        stats: Default::default(),
        no_sensor: !has_sensor,
    };
    write_marker(&d, &view, &pc);
    let stop = CancelToken::new();
    *slot = Some(StabSession {
        view: view.clone(),
        stop: stop.clone(),
        state: pc,
    });
    drop(slot);
    tracing::info!(
        "тест стабільності: {}, {seconds} с{}",
        kind.as_str(),
        if is_night(seconds) { " (ніч)" } else { "" }
    );
    let thread_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("optimax-stability".into())
        .spawn(move || {
            monitor(
                &thread_app,
                child,
                stdin,
                &messages,
                &stop,
                gpu,
                has_sensor,
                &d,
            )
        });
    if let Err(e) = spawned {
        *lock(&state.stability) = None;
        return Err(CmdError::Failed(format!("потік тесту не запустився: {e}")));
    }
    Ok(view)
}

fn read_messages(child: &mut Child) -> CmdResult<Receiver<WorkerMessage>> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CmdError::Failed("немає виводу процесу тесту".into()))?;
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("optimax-stability-read".into())
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(m) = serde_json::from_str::<WorkerMessage>(&line) {
                    if tx.send(m).is_err() {
                        break;
                    }
                }
            }
        })
        .map_err(failed)?;
    Ok(rx)
}

/// Стежить за тестом до кінця: повідомлення процесу, температура, зависання, зупинка.
#[allow(clippy::too_many_arguments)]
fn monitor(
    app: &AppHandle,
    mut child: Child,
    stdin: Option<ChildStdin>,
    messages: &Receiver<WorkerMessage>,
    stop: &CancelToken,
    mut gpu: Option<NvmlGpu>,
    has_sensor: bool,
    d: &Path,
) {
    let started = Instant::now();
    // Довгий тест без нагляду не має перериватися сном. Діє, поки живе цей потік.
    let awake = optimax_stability::awake::KeepAwake::start();
    if awake.is_none() {
        tracing::warn!("не вдалося заборонити сон на час тесту — ПК може заснути");
    }
    let mut guard = Guard::new(has_sensor);
    let mut acc = StatsAcc::default();
    let mut last_message = Instant::now();
    let mut last_sample = Instant::now() - Duration::from_secs(1);
    let mut last_marker = Instant::now();
    let state = app.state::<AppState>();
    let update = |f: &mut dyn FnMut(&mut StabView)| -> Option<StabView> {
        let mut guard = lock(&state.stability);
        let s = guard.as_mut()?;
        f(&mut s.view);
        Some(s.view.clone())
    };
    let end = loop {
        let mut finished = None;
        while let Ok(m) = messages.try_recv() {
            last_message = Instant::now();
            match m {
                WorkerMessage::Started {
                    adapter,
                    vram_mb,
                    tested_mb,
                } => {
                    tracing::info!("тест стабільності: {adapter}, {tested_mb} МБ із {vram_mb} МБ");
                    update(&mut |v| {
                        v.adapter.clone_from(&adapter);
                        v.vram_mb = vram_mb;
                        v.tested_mb = tested_mb;
                    });
                }
                WorkerMessage::Progress {
                    elapsed_s,
                    rounds,
                    checked,
                    mismatches,
                } => {
                    let planned = update(&mut |v| {
                        if mismatches > v.mismatches {
                            tracing::warn!("тест стабільності: розбіжностей {mismatches}");
                        }
                        v.elapsed_s = elapsed_s;
                        v.rounds = rounds;
                        v.checked = checked;
                        v.mismatches = mismatches;
                    })
                    .map_or(0, |v| v.planned_s);
                    if stop_on_error(planned, mismatches) {
                        tracing::info!("нічний тест зупинено на першій помилці");
                        finished = Some(StabEnd::Completed);
                    }
                }
                WorkerMessage::Finished {
                    elapsed_s,
                    rounds,
                    checked,
                    mismatches,
                } => {
                    update(&mut |v| {
                        v.elapsed_s = elapsed_s;
                        v.rounds = rounds;
                        v.checked = checked;
                        v.mismatches = mismatches;
                    });
                    finished = Some(StabEnd::Completed);
                }
                WorkerMessage::Cores {
                    current,
                    bad,
                    total,
                } => {
                    update(&mut |v| {
                        if bad.len() > v.bad_cores.len() {
                            tracing::warn!("тест стабільності: помилялися ядра {bad:?}");
                        }
                        v.current_core = current;
                        v.bad_cores.clone_from(&bad);
                        v.total_cores = total;
                    });
                }
                WorkerMessage::CoreHung { core } => {
                    tracing::warn!("тест стабільності: ядро {core} перестало відповідати");
                    update(&mut |v| {
                        v.current_core = Some(core);
                        if !v.bad_cores.contains(&core) {
                            v.bad_cores.push(core);
                        }
                    });
                    finished = Some(StabEnd::WorkerDied);
                }
                WorkerMessage::DeviceLost { code } => {
                    tracing::warn!("тест стабільності: відеокарту втрачено ({code:#010x})");
                    finished = Some(StabEnd::DeviceLost);
                }
                WorkerMessage::Failed { message } => {
                    tracing::warn!("тест стабільності не виконано: {message}");
                    finished = Some(StabEnd::Failed);
                }
            }
        }
        // Зупинив користувач, а процес устиг сказати «готово»: це все одно зупинка.
        if stop.is_cancelled() {
            break StabEnd::Cancelled;
        }
        if let Some(end) = finished {
            break end;
        }
        if last_sample.elapsed() >= Duration::from_secs(1) {
            last_sample = Instant::now();
            let (temp, busiest) = read_sensors(&mut gpu);
            let (clock, power, util) = busiest.map_or((None, None, None), |s| {
                (s.core_clock_mhz, s.power_w, s.utilization_percent)
            });
            acc.add(temp, clock, power, util);
            let stats = acc.stats();
            let view = update(&mut |v| {
                v.temp_c = temp;
                v.clock_mhz = clock;
                v.power_w = power;
                v.util_pct = util;
                v.stats = stats;
            });
            if let Some(view) = view {
                let _ = app.emit("stab-update", &view);
                if last_marker.elapsed() >= Duration::from_secs(5) && !stop.is_cancelled() {
                    last_marker = Instant::now();
                    let pc = lock(&state.stability)
                        .as_ref()
                        .and_then(|s| s.state.clone());
                    write_marker(d, &view, &pc);
                }
            }
            match guard.check(temp, started.elapsed().as_secs_f64()) {
                Some(GuardStop::OverTemp) => break StabEnd::OverTemp,
                Some(GuardStop::SensorLost) => break StabEnd::SensorLost,
                None => {}
            }
        }
        if !matches!(child.try_wait(), Ok(None)) {
            // Останні повідомлення могли прийти разом із виходом.
            std::thread::sleep(Duration::from_millis(200));
            if messages
                .try_iter()
                .any(|m| matches!(m, WorkerMessage::Finished { .. }))
            {
                break StabEnd::Completed;
            }
            break StabEnd::WorkerDied;
        }
        if last_message.elapsed().as_secs_f64() > HANG_S {
            tracing::warn!("тест стабільності: процес не відповідає {HANG_S} с");
            break StabEnd::WorkerDied;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    finish_child(&mut child, stdin);
    drop(awake);
    std::thread::sleep(EVENTS_SETTLE);
    let Some(session) = lock(&state.stability).take() else {
        return;
    };
    let events = events_since(session.view.started_unix_ms);
    let record = session.view.into_record(end, events);
    save(d, &record, session.state);
    remove_marker(d);
    tracing::info!(
        "тест стабільності завершено: {:?}, розбіжностей {}",
        record.outcome,
        record.mismatches
    );
    let _ = app.emit("stab-done", &record);
}

/// Просить процес зупинитись (закритий stdin) і за потреби завершує його примусово.
fn finish_child(child: &mut Child, stdin: Option<ChildStdin>) {
    drop(stdin);
    let deadline = Instant::now() + STOP_WAIT;
    while Instant::now() < deadline {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Зупиняє тест; результат прийде подією `stab-done`.
#[tauri::command]
pub fn stab_stop(state: State<'_, AppState>) {
    if let Some(s) = lock(&state.stability).as_ref() {
        s.stop.cancel();
    }
}

/// Хід тесту, що йде.
#[tauri::command]
pub fn stab_state(state: State<'_, AppState>) -> Option<StabView> {
    lock(&state.stability).as_ref().map(|s| s.view.clone())
}

/// Зупинка при виході з програми: процес завершиться сам (закриється його stdin). Позначка не
/// потрібна: користувач сам закрив програму.
pub fn cancel_session(state: &AppState) {
    if let Some(s) = lock(&state.stability).as_ref() {
        s.stop.cancel();
        if let Ok(d) = dir() {
            remove_marker(&d);
        }
    }
}

// ---- Журнал Windows ----

/// Рахує події, що сталися після `since_unix_s`.
fn count_events(events: &[SystemEvent], since_unix_s: i64) -> StabEvents {
    let mut out = StabEvents {
        read: true,
        ..StabEvents::default()
    };
    for e in events.iter().filter(|e| e.unix_s >= since_unix_s) {
        if e.provider.eq_ignore_ascii_case(DISPLAY_PROVIDER) && e.id == TDR_EVENT_ID {
            out.tdr += 1;
        } else if e.provider.eq_ignore_ascii_case(KERNEL_POWER_PROVIDER)
            && e.id == UNEXPECTED_OFF_EVENT_ID
        {
            out.unexpected_off += 1;
        } else if e.provider.eq_ignore_ascii_case(WHEA_PROVIDER) {
            match whea_kind(e.id) {
                Some((WheaSeverity::Fatal, _)) => out.whea_fatal += 1,
                Some((WheaSeverity::Corrected, _)) => out.whea_corrected += 1,
                None => {}
            }
        }
    }
    out
}

fn events_since(started_unix_ms: i64) -> StabEvents {
    // Позначка могла пролежати довго (ПК вимкнено), тож беремо запас у днях.
    let days = u32::try_from((now_ms() - started_unix_ms) / 86_400_000 + 2).unwrap_or(30);
    match WindowsProbe::new().crash_events(days) {
        Ok(Some(events)) => count_events(&events, started_unix_ms / 1000),
        Ok(None) => StabEvents::default(),
        Err(e) => {
            tracing::warn!("журнал подій не прочитано: {e}");
            StabEvents::default()
        }
    }
}

// ---- Позначка й збережені результати ----

fn write_marker(d: &Path, view: &StabView, state: &Option<SystemState>) {
    let run = || -> std::io::Result<()> {
        let tmp = d.join("running.tmp");
        let marker = Marker {
            view: view.clone(),
            state: state.clone(),
        };
        std::fs::write(&tmp, serde_json::to_vec(&marker)?)?;
        std::fs::rename(&tmp, d.join(MARKER))
    };
    if let Err(e) = run() {
        tracing::warn!("позначку тесту не записано: {e}");
    }
}

fn remove_marker(d: &Path) {
    let _ = std::fs::remove_file(d.join(MARKER));
}

/// Незавершений тест із позначки стає результатом.
fn recover(d: &Path) {
    let path = d.join(MARKER);
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    if let Ok(Marker { view, state }) = serde_json::from_slice::<Marker>(&bytes) {
        let events = events_since(view.started_unix_ms);
        let record = view.into_record(StabEnd::Interrupted, events);
        tracing::warn!(
            "знайдено незавершений тест стабільності: {:?}",
            record.outcome
        );
        save(d, &record, state);
    }
    let _ = std::fs::remove_file(path);
}

fn save(d: &Path, record: &StabRecord, state: Option<SystemState>) {
    let entry = StabEntry {
        record: record.clone(),
        state,
        changes: Vec::new(),
    };
    let run = || -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(&entry)?;
        std::fs::write(d.join(format!("{}.json", record.id)), json)
    };
    if let Err(e) = run() {
        tracing::warn!("результат тесту не збережено: {e}");
    }
    for old in read_all(d).iter().skip(KEEP_RECORDS) {
        let _ = std::fs::remove_file(d.join(format!("{}.json", old.record.id)));
    }
}

/// Ідентифікатор лише з безпечних символів (він же ім'я файлу).
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn read_all(d: &Path) -> Vec<StabEntry> {
    let Ok(entries) = std::fs::read_dir(d) else {
        return Vec::new();
    };
    let mut out: Vec<StabEntry> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter(|e| e.file_name() != MARKER)
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<StabEntry>(&b).ok())
        .filter(|r| valid_id(&r.record.id))
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.record.started_unix_ms));
    with_changes(&mut out);
    out
}

/// Для кожного тесту — що змінилося в стані ПК від попереднього (старішого) тесту того самого
/// виду, де стан відомий. `entries` — найновіші першими.
fn with_changes(entries: &mut [StabEntry]) {
    for i in 0..entries.len() {
        let changes = entries[i]
            .state
            .as_ref()
            .and_then(|cur| {
                entries[i + 1..]
                    .iter()
                    .find(|e| e.record.kind == entries[i].record.kind && e.state.is_some())
                    .and_then(|prev| prev.state.as_ref())
                    .map(|prev| diff(prev, cur))
            })
            .unwrap_or_default();
        entries[i].changes = changes;
    }
}

/// Збережені результати, найновіші першими (разом із незавершеним тестом, якщо він є).
#[tauri::command]
pub async fn stab_list(app: AppHandle) -> CmdResult<Vec<StabEntry>> {
    blocking(move || {
        let d = dir()?;
        if !is_active(&app.state::<AppState>()) {
            recover(&d);
        }
        Ok(read_all(&d))
    })
    .await
}

/// Видаляє збережений результат.
#[tauri::command]
pub async fn stab_delete(id: String) -> CmdResult<()> {
    blocking(move || {
        if !valid_id(&id) {
            return Err(CmdError::Failed("недопустимий ідентифікатор".into()));
        }
        let path = dir()?.join(format!("{id}.json"));
        if path.is_file() {
            std::fs::remove_file(&path).map_err(failed)?;
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::CivilDate;

    fn event(provider: &str, id: u16, unix_s: i64) -> SystemEvent {
        SystemEvent {
            provider: provider.into(),
            id,
            level: 2,
            date: CivilDate::new(2026, 11, 6).expect("дата"),
            unix_s,
            data: Vec::new(),
        }
    }

    #[test]
    fn only_events_during_the_test_are_counted() {
        let events = [
            event(DISPLAY_PROVIDER, TDR_EVENT_ID, 99),
            event(DISPLAY_PROVIDER, TDR_EVENT_ID, 100),
            event(KERNEL_POWER_PROVIDER, UNEXPECTED_OFF_EVENT_ID, 101),
            event(WHEA_PROVIDER, 19, 102),
            event(WHEA_PROVIDER, 18, 103),
            event(WHEA_PROVIDER, 9999, 104),
            event("Application Error", 1000, 105),
        ];
        let c = count_events(&events, 100);
        assert!(c.read);
        assert_eq!(
            (c.tdr, c.unexpected_off, c.whea_corrected, c.whea_fatal),
            (1, 1, 1, 1)
        );
    }

    fn record(id: &str, kind: &str, started: i64) -> StabRecord {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": kind, "started_unix_ms": started, "adapter": "X",
            "planned_s": 600, "elapsed_s": 600.0, "rounds": 1, "checked": 1, "mismatches": 0,
            "tested_mb": 0, "vram_mb": 0, "outcome": "no_errors",
            "events": { "read": true, "tdr": 0, "whea_corrected": 0, "whea_fatal": 0, "unexpected_off": 0 },
            "stats": { "max_temp_c": null, "avg_clock_mhz": null, "avg_power_w": null, "avg_util_pct": null },
            "low_load": false, "no_sensor": true
        }))
        .expect("запис")
    }

    fn pc(bios: &str, ram: u32) -> Option<SystemState> {
        Some(SystemState {
            bios: Some(bios.into()),
            ram_mts: Some(ram),
            ..SystemState::default()
        })
    }

    #[test]
    fn saved_tests_keep_pc_state_and_show_changes_per_kind() {
        let d = std::env::temp_dir().join(format!("optimax-stab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("тека");
        // Старий запис без стану читається як і раніше.
        let old = serde_json::to_vec(&record("1-ram", "ram", 1)).expect("json");
        std::fs::write(d.join("1-ram.json"), old).expect("старий");
        save(&d, &record("2-ram", "ram", 2), pc("P2.40", 3733));
        save(&d, &record("3-cpu", "cpu", 3), pc("P2.40", 3600));
        save(&d, &record("4-ram", "ram", 4), pc("P2.40", 3600));
        // Позначка тесту, що йде, — не результат.
        std::fs::write(d.join(MARKER), b"{}").expect("позначка");
        let all = read_all(&d);
        let _ = std::fs::remove_dir_all(&d);
        let ids: Vec<&str> = all.iter().map(|e| e.record.id.as_str()).collect();
        assert_eq!(ids, ["4-ram", "3-cpu", "2-ram", "1-ram"]);
        // Пам'ять порівнюється з попереднім тестом пам'яті, а не з тестом процесора.
        assert_eq!(all[0].changes.len(), 1);
        assert_eq!(
            (
                all[0].changes[0].from.as_str(),
                all[0].changes[0].to.as_str()
            ),
            ("3733 MT/s", "3600 MT/s")
        );
        assert!(all[1].changes.is_empty());
        assert!(all[2].changes.is_empty());
        assert!(all[3].state.is_none());
    }

    #[test]
    fn marker_keeps_pc_state_for_an_interrupted_test() {
        let view: StabView = serde_json::from_value(serde_json::json!({
            "id": "5-gpu", "kind": "gpu", "started_unix_ms": 5, "adapter": "X", "planned_s": 14400,
            "elapsed_s": 3.0, "rounds": 0, "checked": 0, "mismatches": 0, "tested_mb": 0, "vram_mb": 0,
            "temp_c": null, "clock_mhz": null, "power_w": null, "util_pct": null, "limit_c": 85,
            "stats": { "max_temp_c": null, "avg_clock_mhz": null, "avg_power_w": null, "avg_util_pct": null },
            "no_sensor": false, "current_core": null, "total_cores": 0
        }))
        .expect("хід");
        let bytes = serde_json::to_vec(&Marker {
            view,
            state: pc("P2.40", 3600),
        })
        .expect("json");
        let back: Marker = serde_json::from_slice(&bytes).expect("назад");
        assert_eq!(back.view.planned_s, 14400);
        assert_eq!(back.state, pc("P2.40", 3600));
        // Позначка зі старої версії (лише хід) теж читається.
        let only_view = serde_json::to_vec(&back.view).expect("json");
        assert!(serde_json::from_slice::<Marker>(&only_view)
            .expect("стара")
            .state
            .is_none());
    }

    #[test]
    fn ids_cannot_leave_the_folder() {
        assert!(valid_id("1793985592398-gpu"));
        for bad in ["", "..", "a/b", "a\\b", "a.json", "a b"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }
}
