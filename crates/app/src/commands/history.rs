//! Історія записів «Діагностики»: збереження після кожного запису, список, порівняння двох
//! записів, повторний аналіз старого запису за поточними правилами, видалення.
//! Усе локально в захищеній теці даних (`…\history`); нічого не надсилається.

use super::blocking;
use crate::dto::DiagnosisResult;
use crate::error::{CmdError, CmdResult};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use optimax_audit::model::GpuVendor;
use optimax_audit::{SystemProbe, WindowsProbe};
use optimax_diagnosis::timeline::Timeline;
use optimax_diagnosis::{diagnose, Bottleneck, Context};
use optimax_frametime::Recording;
use optimax_history::{
    compare, diff, driver_effects, entry_id, fps_noise_pct, regressions, DisplayState,
    DriverEffect, FileStamp, HistoryEntry, RecordComparison, Regression, StateChange, StdDir,
    Store, SystemState, KEEP_RECORDINGS_PER_GAME,
};
use serde::Serialize;
use std::io::{Read, Write};

/// Рядок історії: запис і що змінилося в стані ПК від попереднього запису тієї ж гри.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HistoryRow {
    /// Запис.
    pub entry: HistoryEntry,
    /// Зміни відносно попереднього (старішого) запису цієї гри; порожньо для першого.
    pub changes: Vec<StateChange>,
}

/// Список історії для екрана.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HistoryView {
    /// Записи, найновіші першими.
    pub rows: Vec<HistoryRow>,
    /// Файлів, які не вдалося прочитати (пошкоджені або з несумісної версії).
    pub broken: u32,
}

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

fn dir() -> CmdResult<StdDir> {
    let base = optimax_core::datadir::ui_dir().map_err(failed)?;
    StdDir::new(base.join("history")).map_err(failed)
}

/// Назва значення enum так, як її серіалізує serde (`enabled`, `not_set`…).
fn serde_name(v: impl Serialize) -> Option<String> {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

/// Розмір і час зміни exe запущеної гри (оновлення гри змінює хоча б одне з них).
fn game_file(game: &str) -> Option<FileStamp> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let exe = sys
        .processes()
        .values()
        .find(|p| p.name().to_string_lossy().eq_ignore_ascii_case(game))?
        .exe()?
        .to_path_buf();
    let meta = std::fs::metadata(exe).ok()?;
    let modified_unix = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(FileStamp {
        bytes: meta.len(),
        modified_unix: i64::try_from(modified_unix).ok()?,
    })
}

/// Знімок стану ПК (лише читання; невдалі частини лишаються `None`).
pub fn system_state(game: &str) -> SystemState {
    let probe = WindowsProbe::new();
    // Дискретна відеокарта зазвичай та, що з найбільшою відеопам'яттю.
    let gpu = probe
        .gpus()
        .ok()
        .and_then(|g| g.into_iter().max_by_key(|a| a.vram_mib.unwrap_or(0)));
    let display = probe.displays().ok().and_then(|d| {
        d.into_iter().find(|d| d.primary).map(|d| DisplayState {
            width: d.width,
            height: d.height,
            hz: d.current_hz,
        })
    });
    let firmware = probe.firmware().ok();
    SystemState {
        optimax: env!("CARGO_PKG_VERSION").to_string(),
        windows: sysinfo::System::long_os_version(),
        windows_build: sysinfo::System::kernel_version(),
        gpu: gpu.as_ref().map(|g| g.name.clone()),
        // NVIDIA — у звичному форматі («610.62»), як в аудиті; інші — як подає Windows.
        driver: gpu.and_then(|g| {
            let raw = g.driver_version?;
            Some(
                match (g.vendor, optimax_audit::gpu_info::nvidia_version(&raw)) {
                    (GpuVendor::Nvidia, Some(nv)) => nv,
                    _ => raw,
                },
            )
        }),
        display,
        power_plan: probe.power_plan().ok().map(|p| p.name.unwrap_or(p.guid)),
        bios: firmware
            .as_ref()
            .and_then(|f| f.bios.as_ref().map(|b| b.version.clone())),
        ram_mts: firmware
            .as_ref()
            .and_then(|f| f.memory.iter().filter_map(|m| m.configured_mts).max()),
        hags: probe.hags().ok().and_then(|h| serde_name(h.registry)),
        game_mode: probe.game_mode().ok().and_then(serde_name),
        game_file: game_file(game),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Підсумок запису для історії (`None` — у записі немає висновку: «недостатньо даних»).
pub fn entry_from(
    r: &DiagnosisResult,
    rec: &Recording,
    state: SystemState,
    scene: Option<String>,
    at_unix_ms: i64,
) -> Option<HistoryEntry> {
    let d = &r.diagnosis;
    if d.bottleneck == Bottleneck::InsufficientData {
        return None;
    }
    let head = &d.verdicts.first()?.finding;
    let noise = Timeline::from_recording(rec).and_then(|tl| {
        let ft: Vec<f64> = tl.frames.iter().map(|f| f.frametime_ms).collect();
        fps_noise_pct(&ft)
    });
    Some(HistoryEntry {
        id: entry_id(&d.game, at_unix_ms),
        game: d.game.clone(),
        at_unix_ms,
        bottleneck: d.bottleneck,
        title: head.title.clone(),
        confidence: head.confidence.percent(),
        avg_fps: d.stats.as_ref().map(|s| s.avg_fps),
        low_1pct_fps: d.stats.as_ref().map(|s| s.low_1pct_fps),
        low_01pct_fps: d.stats.as_ref().map(|s| s.low_01pct_fps),
        frames: d.stats.as_ref().map_or(0, |s| s.frames as u64),
        counted_s: r.counted_s,
        fps_noise_pct: noise,
        state,
        has_recording: true,
        scene,
    })
}

pub(crate) fn compress(rec: &Recording) -> CmdResult<Vec<u8>> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&serde_json::to_vec(rec).map_err(failed)?)
        .map_err(failed)?;
    gz.finish().map_err(failed)
}

/// Зберігає завершений запис «Діагностики» в історію (блокує: стан ПК і стиснення).
/// Помилка лише логується — діагностика від історії не залежить.
/// Повертає id збереженого запису (`None` — не збережено).
pub fn remember(r: &DiagnosisResult, rec: &Recording, scene: Option<String>) -> Option<String> {
    let run = || -> CmdResult<Option<String>> {
        let state = system_state(&r.diagnosis.game);
        let Some(entry) = entry_from(r, rec, state, scene, now_ms()) else {
            return Ok(None);
        };
        let files = dir()?;
        let store = Store::new(&files);
        store.save(&entry, Some(&compress(rec)?)).map_err(failed)?;
        store.prune(KEEP_RECORDINGS_PER_GAME).map_err(failed)?;
        Ok(Some(entry.id))
    };
    match run() {
        Ok(Some(id)) => {
            tracing::info!("запис додано в історію ({id})");
            Some(id)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("не вдалося зберегти запис в історію: {}", e.message());
            None
        }
    }
}

/// Історія записів, найновіші першими.
#[tauri::command]
pub async fn history_list() -> CmdResult<HistoryView> {
    blocking(|| {
        let files = dir()?;
        let (entries, broken) = Store::new(&files).list().map_err(failed)?;
        Ok(HistoryView {
            rows: rows(entries),
            broken: u32::try_from(broken).unwrap_or(u32::MAX),
        })
    })
    .await
}

/// Рядки з різницею до попереднього запису тієї ж гри (`entries` — найновіші першими).
fn rows(entries: Vec<HistoryEntry>) -> Vec<HistoryRow> {
    let mut out: Vec<HistoryRow> = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let previous = entries[i + 1..]
            .iter()
            .find(|p| p.game.eq_ignore_ascii_case(&e.game));
        out.push(HistoryRow {
            changes: previous
                .map(|p| diff(&p.state, &e.state))
                .unwrap_or_default(),
            entry: e.clone(),
        });
    }
    out
}

fn get(store: &Store<'_>, id: &str) -> CmdResult<HistoryEntry> {
    store
        .get(id)
        .map_err(failed)?
        .ok_or_else(|| CmdError::NotFound(format!("запису {id} немає в історії")))
}

/// Порівняння двох записів (порядок не важливий).
#[tauri::command]
pub async fn history_compare(a: String, b: String) -> CmdResult<RecordComparison> {
    blocking(move || {
        let files = dir()?;
        let store = Store::new(&files);
        Ok(compare(&get(&store, &a)?, &get(&store, &b)?))
    })
    .await
}

/// Порівняння запису `id` з попереднім записом тієї самої гри (`None` — це перший запис гри).
/// Який запис попередній, вирішує [`optimax_history::previous_of`].
#[tauri::command]
pub async fn history_vs_previous(id: String) -> CmdResult<Option<RecordComparison>> {
    blocking(move || {
        let files = dir()?;
        let (entries, _) = Store::new(&files).list().map_err(failed)?;
        let cur = entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| CmdError::NotFound(format!("запису {id} немає в історії")))?;
        Ok(optimax_history::previous_of(&entries, &id).map(|prev| compare(prev, cur)))
    })
    .await
}

/// Старий запис, проаналізований заново поточними правилами (радар, вердикти). Аудит
/// конфігурації не підставляється — він описував би сьогоднішній ПК, а не тодішній.
#[tauri::command]
pub async fn history_open(id: String) -> CmdResult<DiagnosisResult> {
    blocking(move || {
        let files = dir()?;
        let store = Store::new(&files);
        let entry = get(&store, &id)?;
        let bytes = store
            .recording(&id)
            .map_err(failed)?
            .ok_or_else(|| CmdError::NotFound("повний запис уже не зберігається".into()))?;
        let mut json = Vec::new();
        GzDecoder::new(bytes.as_slice())
            .read_to_end(&mut json)
            .map_err(failed)?;
        let rec: Recording = serde_json::from_slice(&json).map_err(failed)?;
        let display_hz = entry.state.display.map(|d| d.hz);
        let ctx = Context {
            display_hz,
            ..Context::default()
        };
        Ok(DiagnosisResult {
            diagnosis: diagnose(&rec, &ctx),
            display_hz,
            presentmon_messages: rec.presentmon_messages.clone(),
            cancelled: rec.cancelled,
            counted_s: rec.counted_seconds(),
            end_reason: rec.end_reason,
        })
    })
    .await
}

/// Ігри, де FPS помітно впав між останнім записом і попереднім порівнянним (для «Головної»).
#[tauri::command]
pub async fn history_regressions() -> CmdResult<Vec<Regression>> {
    blocking(|| {
        let files = dir()?;
        let (entries, _) = Store::new(&files).list().map_err(failed)?;
        Ok(regressions(&entries))
    })
    .await
}

/// Ефект останнього оновлення драйвера по іграх (без мережі — лише записи історії).
#[tauri::command]
pub async fn history_driver_effects() -> CmdResult<Vec<DriverEffect>> {
    blocking(|| {
        let files = dir()?;
        let (entries, _) = Store::new(&files).list().map_err(failed)?;
        Ok(driver_effects(&entries))
    })
    .await
}

/// Видаляє запис з історії.
#[tauri::command]
pub async fn history_delete(id: String) -> CmdResult<()> {
    blocking(move || {
        let files = dir()?;
        Store::new(&files).delete(&id).map_err(failed)?;
        tracing::info!("запис {id} видалено з історії");
        Ok(())
    })
    .await
}

/// Видаляє всю історію; повертає кількість записів.
#[tauri::command]
pub async fn history_clear() -> CmdResult<u32> {
    blocking(|| {
        let files = dir()?;
        let n = Store::new(&files).clear().map_err(failed)?;
        tracing::info!("історію записів очищено ({n})");
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::{Category, FindingBuilder, Impact};
    use optimax_diagnosis::Diagnosis;
    use optimax_frametime::{EndReason, Target};

    fn rec() -> Recording {
        Recording {
            target: Target::ProcessName("re9.exe".into()),
            qpc_frequency: 1000,
            start_qpc: 0,
            frames: vec![],
            snapshots: vec![],
            skipped_lines: 0,
            presentmon_messages: vec![],
            cancelled: false,
            segments: vec![],
            end_reason: EndReason::Completed,
            start_unix_ms: None,
            dpc_trace: None,
            threads: None,
        }
    }

    fn result(bottleneck: Bottleneck) -> DiagnosisResult {
        DiagnosisResult {
            diagnosis: Diagnosis {
                game: "re9.exe".into(),
                stats: None,
                bottleneck,
                verdicts: vec![FindingBuilder::new(
                    "diag",
                    "summary.gpu",
                    Category::Summary,
                    Impact::Info,
                    85,
                )
                .build()
                .into()],
                chart: None,
                frame_gen: None,
                anatomy: None,
                latency: None,
                view: None,
            },
            display_hz: Some(200),
            presentmon_messages: vec![],
            cancelled: false,
            counted_s: 60.0,
            end_reason: EndReason::Completed,
        }
    }

    #[test]
    fn entry_takes_summary_and_skips_insufficient_data() {
        let e = entry_from(
            &result(Bottleneck::Gpu),
            &rec(),
            SystemState::default(),
            None,
            42,
        )
        .unwrap();
        assert_eq!(e.id, "42-re9.exe");
        assert_eq!(e.confidence, 85);
        assert_eq!(e.title.key, "diag.summary.gpu.title");
        assert!(e.fps_noise_pct.is_none());
        assert!(entry_from(
            &result(Bottleneck::InsufficientData),
            &rec(),
            SystemState::default(),
            None,
            42
        )
        .is_none());
    }

    #[test]
    fn rows_diff_against_previous_record_of_the_same_game() {
        let mk = |game: &str, at: i64, driver: &str| {
            let mut e = entry_from(
                &result(Bottleneck::Gpu),
                &rec(),
                SystemState::default(),
                None,
                at,
            )
            .unwrap();
            e.game = game.into();
            e.state.driver = Some(driver.into());
            e
        };
        // Найновіші першими: re9 (572) → cs2 (572) → re9 (566).
        let r = rows(vec![
            mk("re9.exe", 3, "572"),
            mk("cs2.exe", 2, "572"),
            mk("re9.exe", 1, "566"),
        ]);
        assert_eq!(r[0].changes.len(), 1);
        assert_eq!(r[0].changes[0].from, "566");
        assert!(r[1].changes.is_empty(), "у cs2 немає попереднього запису");
        assert!(r[2].changes.is_empty());
    }

    #[test]
    fn recording_compresses_and_reads_back() {
        let bytes = compress(&rec()).unwrap();
        let mut json = Vec::new();
        GzDecoder::new(bytes.as_slice())
            .read_to_end(&mut json)
            .unwrap();
        let back: Recording = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.target, rec().target);
    }
}
