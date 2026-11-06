//! Порівняння 2–4 варіантів будь-якої зміни (науковий A/B, рушій — `optimax_experiment`).
//!
//! Змінює налаштування сам користувач (у грі, драйвері, BIOS): програма лише веде прогони за
//! порядком ABBA (для кількох варіантів — ABC CBA…), підказує в інтерфейсі й оверлеї, який
//! варіант має бути ввімкнений, і після кожного кола рахує, чи висновок уже певний. Перед першим прогоном — прогрів до сталої
//! температури GPU. Підсумок зберігається в теці даних (`experiments\*.json`). Нічого в системі
//! не змінює. Подія `exp-progress` — хід прогону, `exp-update` — новий стан (прогрів).

use super::{blocking, record_game};
use crate::dto::{ExpPhase, ExpRecord, ExpView, ExpWarmup};
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_core::Text;
use optimax_diagnosis::latency::latency_summary;
use optimax_diagnosis::{diagnose, Bottleneck, Context, Timeline};
use optimax_experiment::thermal::{thermal_state, WARMUP_MAX_S};
use optimax_experiment::{
    evaluate, letter, next_variant, Policy, Report, Run, Status, Variant, MAX_VARIANTS,
};
use optimax_frametime::{CancelToken, Recording, StartMode, Target};
use optimax_metrics::{GpuApi, NvmlGpu};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

/// Прогрів за часом, якщо температура GPU недоступна (не NVIDIA), с.
const NO_TEMP_WARMUP_S: f64 = 120.0;
/// Як часто брати температуру під час прогріву.
const WARMUP_STEP: Duration = Duration::from_secs(2);
/// Найдовша назва варіанта, символів.
const MAX_NAME: usize = 32;
/// Скільки збережених порівнянь тримати.
const KEEP_RECORDS: usize = 50;

struct WarmupState {
    samples: Vec<(f64, f64)>,
    elapsed_s: f64,
    has_temp: bool,
    cancel: CancelToken,
}

/// Сесія порівняння.
pub struct ExpSession {
    game: String,
    names: Vec<String>,
    seconds: u32,
    start: StartMode,
    sound: bool,
    policy: Policy,
    runs: Vec<Run>,
    report: Report,
    phase: ExpPhase,
    warmup: Option<WarmupState>,
    /// Варіант прогону, що йде.
    running_arm: Option<Variant>,
    /// Скасування прогону, що йде.
    cancel: Option<CancelToken>,
    saved_id: Option<String>,
}

impl ExpSession {
    fn view(&self) -> ExpView {
        ExpView {
            game: self.game.clone(),
            names: self.names.clone(),
            seconds: self.seconds,
            start: self.start,
            phase: self.phase,
            warmup: self.warmup.as_ref().map(|w| ExpWarmup {
                elapsed_s: w.elapsed_s,
                max_s: if w.has_temp {
                    WARMUP_MAX_S
                } else {
                    NO_TEMP_WARMUP_S
                },
                thermal: thermal_state(&w.samples),
                has_temp: w.has_temp,
            }),
            report: self.report.clone(),
            max_per_arm: u32::try_from(self.policy.max_per_arm).unwrap_or(u32::MAX),
            saved_id: self.saved_id.clone(),
        }
    }

    fn name(&self, v: Variant) -> &str {
        self.names.get(usize::from(v)).map_or("?", String::as_str)
    }

    fn reevaluate(&mut self) {
        self.report = evaluate(&self.runs, &self.policy);
        if self.report.status == Status::Done {
            self.phase = ExpPhase::Done;
        } else if self.phase == ExpPhase::Done {
            // Виключення прогону після висновку знову відкриває тест.
            self.phase = ExpPhase::Ready;
        }
    }
}

/// Назва варіанта: без керівних символів, обрізана; порожня — типова.
fn clean_name(raw: &str, fallback: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect::<String>()
        .trim()
        .to_string();
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

/// Сесія є (на будь-якому етапі): інші записи гри тоді не запускаються.
pub fn is_active(state: &AppState) -> bool {
    lock(&state.experiment).is_some()
}

/// Помилка «іде порівняння» для інших записів.
pub fn busy_error() -> CmdError {
    CmdError::Failed("іде порівняння варіантів — заверши або закрий його".into())
}

/// Починає порівняння.
#[tauri::command]
pub fn exp_start(
    app: AppHandle,
    state: State<'_, AppState>,
    exe: String,
    names: Vec<String>,
    seconds: u32,
    warmup: bool,
) -> CmdResult<ExpView> {
    Target::ProcessName(exe.clone()).validate()?;
    if !(2..=MAX_VARIANTS).contains(&names.len()) {
        return Err(CmdError::Failed(format!(
            "порівнюється від 2 до {MAX_VARIANTS} варіантів"
        )));
    }
    if super::lag::is_watching(&state) {
        return Err(super::lag::busy_error());
    }
    if lock(&state.recording).is_some()
        || lock(&state.ab).is_some()
        || lock(&state.res_test).is_some()
    {
        return Err(CmdError::Failed(
            "уже йде запис, A/B-тест або тест роздільності — дочекайся кінця".into(),
        ));
    }
    let mut slot = lock(&state.experiment);
    if slot.is_some() {
        return Err(busy_error());
    }
    let (start, sound) = {
        let st = lock(&state.settings);
        (st.record_start, st.record_sound)
    };
    let policy = Policy::default().with_variants(names.len()).with_latency();
    let names: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let v = Variant::try_from(i).unwrap_or(0);
            clean_name(n, &letter(v).to_string())
        })
        .collect();
    let warm = warmup.then(|| WarmupState {
        samples: Vec::new(),
        elapsed_s: 0.0,
        has_temp: false,
        cancel: CancelToken::new(),
    });
    let token = warm.as_ref().map(|w| w.cancel.clone());
    let session = ExpSession {
        game: exe,
        names,
        seconds: seconds.clamp(20, 120),
        start,
        sound,
        policy,
        report: evaluate(&[], &policy),
        runs: Vec::new(),
        phase: if warmup {
            ExpPhase::Warmup
        } else {
            ExpPhase::Ready
        },
        warmup: warm,
        running_arm: None,
        cancel: None,
        saved_id: None,
    };
    let view = session.view();
    tracing::info!(
        "порівняння {}: {}, прогін {} с",
        session.game,
        session.names.join(" / "),
        session.seconds
    );
    *slot = Some(session);
    drop(slot);
    if let Some(token) = token {
        spawn_warmup(app, token)?;
    }
    Ok(view)
}

fn spawn_warmup(app: AppHandle, token: CancelToken) -> CmdResult<()> {
    std::thread::Builder::new()
        .name("optimax-exp-warmup".into())
        .spawn(move || warmup_loop(&app, &token))
        .map(|_| ())
        .map_err(|e| CmdError::Failed(format!("прогрів не запустився: {e}")))
}

fn warmup_loop(app: &AppHandle, token: &CancelToken) {
    let mut gpu: Option<Box<dyn GpuApi>> = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
    let started = Instant::now();
    loop {
        // Чекаємо крок дрібними відрізками, щоб скасування спрацьовувало одразу.
        let step_end = Instant::now() + WARMUP_STEP;
        while Instant::now() < step_end {
            if token.is_cancelled() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let temp = gpu.as_mut().and_then(|g| {
            g.sample()
                .iter()
                .filter_map(|s| s.temperature_c)
                .max()
                .map(f64::from)
        });
        let elapsed = started.elapsed().as_secs_f64();
        let state = app.state::<AppState>();
        let view = {
            let mut guard = lock(&state.experiment);
            let Some(s) = guard.as_mut() else {
                return;
            };
            if s.phase != ExpPhase::Warmup {
                return;
            }
            let Some(w) = s.warmup.as_mut() else {
                return;
            };
            w.elapsed_s = elapsed;
            if let Some(t) = temp {
                w.samples.push((elapsed, t));
                w.has_temp = true;
            }
            let th = thermal_state(&w.samples);
            let done = if w.has_temp {
                th.stable || elapsed >= WARMUP_MAX_S
            } else {
                elapsed >= NO_TEMP_WARMUP_S
            };
            if done {
                s.phase = ExpPhase::Ready;
                tracing::info!(
                    "порівняння: прогрів завершено за {elapsed:.0} с ({})",
                    th.slope_c_per_min
                        .map_or_else(|| "без температури".into(), |v| format!("{v:+.2} °C/хв"))
                );
            }
            s.view()
        };
        let _ = app.emit("exp-update", &view);
        if view.phase != ExpPhase::Warmup {
            return;
        }
    }
}

/// Пропустити прогрів (з попередженням у звіті, якщо дрейф буде помітний).
#[tauri::command]
pub fn exp_skip_warmup(state: State<'_, AppState>) -> CmdResult<ExpView> {
    let mut guard = lock(&state.experiment);
    let s = guard
        .as_mut()
        .ok_or_else(|| CmdError::NotFound("порівняння не запущено".into()))?;
    if s.phase == ExpPhase::Warmup {
        if let Some(w) = &s.warmup {
            w.cancel.cancel();
        }
        s.phase = ExpPhase::Ready;
        tracing::info!("порівняння: прогрів пропущено");
    }
    Ok(s.view())
}

/// Поточний стан (якщо порівняння є).
#[tauri::command]
pub fn exp_state(state: State<'_, AppState>) -> Option<ExpView> {
    lock(&state.experiment).as_ref().map(ExpSession::view)
}

/// Прогін одного варіанта з запису (для всіх тестів на рушії порівнянь).
pub(crate) fn run_from(rec: &Recording, variant: Variant) -> CmdResult<Run> {
    let tl = Timeline::from_recording(rec)
        .ok_or_else(|| CmdError::Failed("у прогоні немає кадрів гри".into()))?;
    let d = diagnose(rec, &Context::default());
    let temps: Vec<f64> = rec
        .snapshots
        .iter()
        .filter_map(|s| s.gpus.iter().filter_map(|g| g.temperature_c).max())
        .map(f64::from)
        .collect();
    Ok(Run {
        variant,
        frametimes_ms: tl.frames.iter().map(|f| f.frametime_ms).collect(),
        gpu_temp_avg_c: (!temps.is_empty()).then(|| temps.iter().sum::<f64>() / temps.len() as f64),
        pc_latency_ms: latency_summary(&tl.frames)
            .and_then(|l| l.reliable_pc_latency())
            .map(|s| s.median_ms),
        frame_capped: d.bottleneck == Bottleneck::FrameCap,
        excluded: false,
    })
}

/// Записує наступний прогін (варіант — за порядком рушія) і перераховує висновок.
#[tauri::command]
pub async fn exp_run_next(app: AppHandle) -> CmdResult<ExpView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (arm, game, seconds, start, sound, cancel) = {
            let mut guard = lock(&state.experiment);
            let s = guard
                .as_mut()
                .ok_or_else(|| CmdError::NotFound("порівняння не запущено".into()))?;
            match s.phase {
                ExpPhase::Ready => {}
                ExpPhase::Warmup => {
                    return Err(CmdError::Failed(
                        "іде прогрів — дочекайся його кінця або пропусти".into(),
                    ))
                }
                ExpPhase::Running => return Err(CmdError::Failed("прогін уже йде".into())),
                ExpPhase::Done => {
                    return Err(CmdError::Failed(
                        "висновок уже є — прогонів більше не треба".into(),
                    ))
                }
            }
            let arm = next_variant(&s.runs, s.policy.variants);
            let cancel = CancelToken::new();
            s.cancel = Some(cancel.clone());
            s.running_arm = Some(arm);
            s.phase = ExpPhase::Running;
            (arm, s.game.clone(), s.seconds, s.start, s.sound, cancel)
        };
        let measured = record_game(&app, game, seconds, start, sound, &cancel, "exp-progress")
            .and_then(|rec| {
                if rec.cancelled {
                    return Err(CmdError::Cancelled("прогін зупинено".into()));
                }
                run_from(&rec, arm)
            });
        let mut guard = lock(&state.experiment);
        let s = guard
            .as_mut()
            .ok_or_else(|| CmdError::NotFound("порівняння закрито під час прогону".into()))?;
        s.cancel = None;
        s.running_arm = None;
        s.phase = ExpPhase::Ready;
        let run = measured?;
        s.runs.push(run);
        s.reevaluate();
        tracing::info!(
            "порівняння {}: прогін {} («{}») записано",
            s.game,
            s.runs.len(),
            s.name(arm)
        );
        if s.phase == ExpPhase::Done && s.saved_id.is_none() {
            s.saved_id = save(s);
        }
        Ok(s.view())
    })
    .await
}

/// Виключити прогін з оцінки або повернути його (порядок прогонів відновлюється повтором).
#[tauri::command]
pub fn exp_exclude(state: State<'_, AppState>, index: u32, excluded: bool) -> CmdResult<ExpView> {
    let mut guard = lock(&state.experiment);
    let s = guard
        .as_mut()
        .ok_or_else(|| CmdError::NotFound("порівняння не запущено".into()))?;
    if s.phase == ExpPhase::Running {
        return Err(CmdError::Failed("іде прогін — дочекайся його кінця".into()));
    }
    let run = s
        .runs
        .get_mut(index as usize)
        .ok_or_else(|| CmdError::NotFound(format!("прогону {} немає", index + 1)))?;
    run.excluded = excluded;
    s.reevaluate();
    if s.phase == ExpPhase::Done {
        // Збережений підсумок оновлюється тим самим ідентифікатором.
        s.saved_id = save(s);
    }
    tracing::info!(
        "порівняння: прогін {} {}",
        index + 1,
        if excluded {
            "виключено"
        } else {
            "повернено"
        }
    );
    Ok(s.view())
}

/// Зупиняє прогін (якщо йде) і закриває порівняння.
#[tauri::command]
pub fn exp_close(state: State<'_, AppState>) {
    cancel_session(&state);
}

/// Те саме, що [`exp_close`], для виклику з коду (вихід із програми).
pub fn cancel_session(state: &AppState) {
    if let Some(s) = lock(&state.experiment).take() {
        if let Some(c) = &s.cancel {
            c.cancel();
        }
        if let Some(w) = &s.warmup {
            w.cancel.cancel();
        }
    }
}

/// Рядок оверлею: який варіант має бути ввімкнений.
pub fn overlay_text(state: &AppState) -> Option<Text> {
    let guard = lock(&state.experiment);
    let s = guard.as_ref()?;
    match s.phase {
        ExpPhase::Warmup => Some(Text::new("overlay.exp.warmup")),
        ExpPhase::Ready => {
            let arm = next_variant(&s.runs, s.policy.variants);
            Some(
                Text::new("overlay.exp.next")
                    .arg("name", s.name(arm))
                    .arg("n", s.runs.len() + 1),
            )
        }
        ExpPhase::Running => s
            .running_arm
            .map(|arm| Text::new("overlay.exp.running").arg("name", s.name(arm))),
        ExpPhase::Done => None,
    }
}

// ---- Збережені порівняння ----

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

fn dir() -> CmdResult<PathBuf> {
    let d = optimax_core::datadir::data_dir()
        .map_err(failed)?
        .join("experiments");
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

/// Ідентифікатор лише з безпечних символів (він же ім'я файлу).
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn record_id(at: i64, game: &str) -> String {
    let g: String = game
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{at}-{g}")
}

/// Зберігає підсумок; помилка лише логується (порівняння від збереження не залежить).
fn save(s: &ExpSession) -> Option<String> {
    let at = now_ms();
    let id = s.saved_id.clone().unwrap_or_else(|| record_id(at, &s.game));
    let rec = ExpRecord {
        id: id.clone(),
        game: s.game.clone(),
        at_unix_ms: at,
        names: s.names.clone(),
        seconds: s.seconds,
        report: s.report.clone(),
    };
    let run = || -> CmdResult<()> {
        let d = dir()?;
        let json = serde_json::to_vec_pretty(&rec).map_err(failed)?;
        std::fs::write(d.join(format!("{id}.json")), json).map_err(failed)?;
        prune(&d);
        Ok(())
    };
    match run() {
        Ok(()) => {
            tracing::info!("порівняння збережено ({id})");
            Some(id)
        }
        Err(e) => {
            tracing::warn!("порівняння не збережено: {}", e.message());
            None
        }
    }
}

/// Підсумок, збережений до порівняння кількох варіантів (два варіанти «A»/«B», оцінки на
/// верхньому рівні звіту), переводиться в теперішній вигляд.
fn upgrade_legacy(v: &mut serde_json::Value) {
    use serde_json::{json, Value};
    let Some(report) = v.get_mut("report").and_then(Value::as_object_mut) else {
        return;
    };
    if report.contains_key("comparisons") {
        return;
    }
    let arm = |x: Option<&Value>| match x.and_then(Value::as_str) {
        Some("B") => json!(1),
        Some("A") => json!(0),
        _ => Value::Null,
    };
    let comparison = json!({
        "variant": 1,
        "verdict": report.get("verdict").cloned().unwrap_or(json!("unclear")),
        "avg_fps": report.get("avg_fps").cloned().unwrap_or(Value::Null),
        "low_1pct": report.get("low_1pct").cloned().unwrap_or(Value::Null),
    });
    let next = arm(report.get("next"));
    report.insert("next".into(), next);
    report.insert("variants".into(), json!(2));
    report.insert("comparisons".into(), json!([comparison]));
    report.insert("more_per_arm".into(), Value::Null);
    report.insert(
        "margin_avg_pct".into(),
        json!(Policy::default().margin_avg_pct),
    );
    report.insert(
        "margin_low_pct".into(),
        json!(Policy::default().margin_low_pct),
    );
    if let Some(runs) = report.get_mut("runs").and_then(Value::as_array_mut) {
        for r in runs.iter_mut().filter_map(Value::as_object_mut) {
            let a = arm(r.get("arm"));
            r.insert("variant".into(), a);
        }
    }
}

fn parse_record(bytes: &[u8]) -> Option<ExpRecord> {
    let mut v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    upgrade_legacy(&mut v);
    serde_json::from_value(v).ok()
}

fn read_all(d: &std::path::Path) -> Vec<ExpRecord> {
    let Ok(entries) = std::fs::read_dir(d) else {
        return Vec::new();
    };
    let mut out: Vec<ExpRecord> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| parse_record(&b))
        .filter(|r| valid_id(&r.id))
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.at_unix_ms));
    out
}

fn prune(d: &std::path::Path) {
    for old in read_all(d).iter().skip(KEEP_RECORDS) {
        let _ = std::fs::remove_file(d.join(format!("{}.json", old.id)));
    }
}

/// Збережені порівняння, найновіші першими.
#[tauri::command]
pub async fn exp_list() -> CmdResult<Vec<ExpRecord>> {
    blocking(|| Ok(read_all(&dir()?))).await
}

/// Видаляє збережене порівняння.
#[tauri::command]
pub async fn exp_delete(id: String) -> CmdResult<()> {
    blocking(move || {
        if !valid_id(&id) {
            return Err(CmdError::Failed("недопустимий ідентифікатор".into()));
        }
        let path = dir()?.join(format!("{id}.json"));
        if path.is_file() {
            std::fs::remove_file(&path).map_err(failed)?;
            tracing::info!("порівняння {id} видалено");
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_ids_are_safe() {
        assert_eq!(clean_name("  DLSS\nQuality  ", "A"), "DLSSQuality");
        assert_eq!(clean_name("   ", "B"), "B");
        assert_eq!(clean_name(&"x".repeat(100), "A").chars().count(), MAX_NAME);
        let id = record_id(1_700_000_000_000, "..\\Re 9.exe");
        assert!(valid_id(&id), "{id}");
        assert!(!id.contains('\\') && !id.contains(' '));
        for bad in ["", "..\\x", "a/b", ".hidden", "a b"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn two_variant_records_from_before_are_upgraded() {
        let old =
            br#"{"id":"1-re9.exe","game":"re9.exe","at_unix_ms":1,"names":["Q","P"],"seconds":30,
          "report":{"status":"done","next":null,"verdict":"better",
            "avg_fps":{"a":112,"b":137,"delta_pct":22.4,"ci_low_pct":13.2,"ci_high_pct":31.7,
              "drift_pct_per_run":0.4,"run_sd_pct":2,"outcome":"better"},
            "low_1pct":null,"more_pairs":null,"confidence_pct":98.58,
            "runs":[{"arm":"A","order":0,"avg_fps":112,"low_1pct_fps":84.7,"frames":3400,
              "duration_s":30,"low_precision_pct":5.3,"excluded":false,"suspect":null},
              {"arm":"B","order":1,"avg_fps":134.4,"low_1pct_fps":90.8,"frames":4000,
              "duration_s":30,"low_precision_pct":7.7,"excluded":true,"suspect":"level"}],
            "warnings":[]}}"#;
        let r = parse_record(old).expect("старий запис читається");
        assert_eq!(r.names, ["Q", "P"]);
        assert_eq!(r.report.variants, 2);
        let c = r.report.comparison(1).expect("порівняння B");
        assert_eq!(c.verdict, optimax_experiment::Outcome::Better);
        assert!((c.avg_fps.expect("оцінка").delta_pct - 22.4).abs() < 1e-9);
        assert_eq!(r.report.runs[1].variant, 1);
        assert!(r.report.runs[1].excluded);
        // Теперішній формат проходить без змін.
        let now = serde_json::to_vec(&r).expect("json");
        assert_eq!(parse_record(&now).map(|x| x.report), Some(r.report));
    }
}
