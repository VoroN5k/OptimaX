//! Головний екран «Стан ПК»: підсумки, які вже обчислили інші частини програми. Нової логіки
//! вердиктів тут немає — лише збір: останній діагноз гри, аудит конфігурації, швидкий скан
//! системного диска за правилами очищення, вік драйвера відеокарти (з аудиту).

use super::blocking;
use crate::commands::cleanup::{last_system_scan, scan_system_drive, ScanView};
use crate::dto::DiagnosisResult;
use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_audit::{run_audit as audit, AuditReport, WindowsProbe};
use optimax_core::{CivilDate, Finding, Impact, Text};
use optimax_diagnosis::Bottleneck;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Файл з підсумком останнього запису гри (у захищеній теці даних).
const LAST_GAME_FILE: &str = "last_game.json";

/// Підсумок останнього запису гри.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LastGame {
    /// Процес гри.
    pub game: String,
    /// Вузьке місце.
    pub bottleneck: Bottleneck,
    /// Заголовок вердикту («re9.exe: вузьке місце — відеокарта»).
    pub title: Text,
    /// Впевненість вердикту, %.
    pub confidence: u8,
    /// Середній FPS.
    pub avg_fps: Option<f64>,
    /// 1% low.
    pub low_1pct_fps: Option<f64>,
    /// Коли записано (Unix-секунди).
    pub at_unix: i64,
}

impl LastGame {
    /// Підсумок із результату діагностики (`None` — у результаті немає вердикту).
    /// Запис без висновку («недостатньо даних») попередній підсумок не замінює.
    pub fn from_result(r: &DiagnosisResult, at_unix: i64) -> Option<Self> {
        if r.diagnosis.bottleneck == Bottleneck::InsufficientData {
            return None;
        }
        let head = &r.diagnosis.verdicts.first()?.finding;
        Some(Self {
            game: r.diagnosis.game.clone(),
            bottleneck: r.diagnosis.bottleneck,
            title: head.title.clone(),
            confidence: head.confidence.percent(),
            avg_fps: r.diagnosis.stats.as_ref().map(|s| s.avg_fps),
            low_1pct_fps: r.diagnosis.stats.as_ref().map(|s| s.low_1pct_fps),
            at_unix,
        })
    }
}

/// Конфігурація: скільки проблем і що з драйвером.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ConfigSummary {
    /// Знахідок із високим впливом.
    pub high: u32,
    /// З помірним.
    pub moderate: u32,
    /// Знахідка про драйвер відеокарти (вік), якщо аудит її дав.
    pub driver: Option<Finding>,
    /// Повний звіт (список нижче на головному екрані).
    pub report: AuditReport,
}

impl ConfigSummary {
    /// Підсумок звіту аудиту.
    pub fn from_report(report: AuditReport) -> Self {
        let count = |i: Impact| {
            u32::try_from(report.findings.iter().filter(|f| f.impact == i).count())
                .unwrap_or(u32::MAX)
        };
        Self {
            high: count(Impact::High),
            moderate: count(Impact::Moderate),
            driver: report
                .findings
                .iter()
                .find(|f| f.id.starts_with("driver."))
                .cloned(),
            report,
        }
    }
}

/// Диск: що можна звільнити за правилами очищення.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DiskSummary {
    /// Корінь (`C:\`).
    pub drive: String,
    /// 🟢 байтів за правилами.
    pub safe_bytes: u64,
    /// 🟡 байтів (лише через карантин).
    pub consequences_bytes: u64,
    /// Вільно на диску.
    pub free_bytes: Option<u64>,
    /// Обсяг диска.
    pub total_bytes: Option<u64>,
    /// Коли сканували (Unix-секунди).
    pub scanned_unix: i64,
}

impl DiskSummary {
    fn from_view(v: &ScanView) -> Self {
        Self {
            drive: v.report.drive.clone(),
            safe_bytes: v.safe_bytes,
            consequences_bytes: v.consequences_bytes,
            free_bytes: v.drive.as_ref().map(|d| d.free_bytes),
            total_bytes: v.drive.as_ref().map(|d| d.total_bytes),
            scanned_unix: v.report.started_unix,
        }
    }
}

/// Кеш головного екрана на час роботи програми: аудит і скан не повторюються при кожному
/// переході на екран.
#[derive(Default)]
pub struct DashCache {
    config: Option<ConfigSummary>,
    disk: Option<DiskSummary>,
}

fn last_game_path() -> CmdResult<PathBuf> {
    let dir = optimax_core::datadir::data_dir().map_err(|e| CmdError::Failed(e.to_string()))?;
    Ok(dir.join(LAST_GAME_FILE))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Запам'ятовує підсумок завершеного запису гри (для головного екрана й після перезапуску).
/// Помилка запису лише логується: діагностика від неї не залежить.
pub fn remember_last_game(r: &DiagnosisResult) {
    let Some(last) = LastGame::from_result(r, now_unix()) else {
        return;
    };
    let saved = last_game_path().and_then(|p| {
        let json = serde_json::to_vec_pretty(&last).map_err(|e| CmdError::Failed(e.to_string()))?;
        std::fs::write(&p, json).map_err(|e| CmdError::Failed(format!("{}: {e}", p.display())))
    });
    if let Err(e) = saved {
        tracing::warn!(
            "не вдалося зберегти підсумок останньої гри: {}",
            e.message()
        );
    }
}

/// Останній запис гри: з файлу (переживає перезапуск). `None` — записів ще не було або файл
/// пошкоджений (тоді плитка просто пропонує записати гру).
#[tauri::command]
pub async fn dashboard_last_game() -> CmdResult<Option<LastGame>> {
    blocking(|| {
        let p = last_game_path()?;
        let Ok(bytes) = std::fs::read(&p) else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(&bytes).ok())
    })
    .await
}

/// Аудит конфігурації (лише читання). Раз за запуск програми; `refresh` — повторити.
#[tauri::command]
pub async fn dashboard_config(app: AppHandle, refresh: bool) -> CmdResult<ConfigSummary> {
    blocking(move || {
        let state = app.state::<AppState>();
        if !refresh {
            if let Some(c) = lock(&state.dashboard).config.clone() {
                return Ok(c);
            }
        }
        let summary = ConfigSummary::from_report(audit(&WindowsProbe::new(), CivilDate::today()));
        lock(&state.dashboard).config = Some(summary.clone());
        Ok(summary)
    })
    .await
}

/// Що можна звільнити на системному диску: останній скан цієї сесії або швидкий скан за
/// правилами (лише розміри, нічого не видаляє). `refresh` — сканувати заново.
#[tauri::command]
pub async fn dashboard_disk(app: AppHandle, refresh: bool) -> CmdResult<DiskSummary> {
    blocking(move || {
        let state = app.state::<AppState>();
        if !refresh {
            // Свіжіший скан з екрана «Очищення» має перевагу над кешем головного.
            if let Some(v) = last_system_scan(&state) {
                let d = DiskSummary::from_view(&v);
                lock(&state.dashboard).disk = Some(d.clone());
                return Ok(d);
            }
            if let Some(d) = lock(&state.dashboard).disk.clone() {
                return Ok(d);
            }
        }
        let v = scan_system_drive(&app)?;
        if v.report.cancelled {
            return Err(CmdError::Cancelled("скан скасовано".into()));
        }
        let d = DiskSummary::from_view(&v);
        lock(&state.dashboard).disk = Some(d.clone());
        Ok(d)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::FindingBuilder;
    use optimax_diagnosis::Diagnosis;

    fn result(verdicts: Vec<Finding>) -> DiagnosisResult {
        DiagnosisResult {
            diagnosis: Diagnosis {
                game: "re9.exe".into(),
                stats: None,
                bottleneck: Bottleneck::Gpu,
                verdicts: verdicts.into_iter().map(Into::into).collect(),
                chart: None,
                frame_gen: None,
            },
            display_hz: Some(200),
            presentmon_messages: vec![],
            cancelled: false,
            counted_s: 30.0,
            end_reason: optimax_frametime::EndReason::Completed,
        }
    }

    fn finding(module: &'static str, id: &'static str, impact: Impact) -> Finding {
        FindingBuilder::new(module, id, optimax_core::Category::Gpu, impact, 81).build()
    }

    #[test]
    fn last_game_takes_the_summary_verdict() {
        let r = result(vec![
            finding("diag", "summary.gpu", Impact::Info),
            finding("diag", "gpu_bound", Impact::Info),
        ]);
        let g = LastGame::from_result(&r, 1_700_000_000).unwrap();
        assert_eq!(g.game, "re9.exe");
        assert_eq!(g.bottleneck, Bottleneck::Gpu);
        assert_eq!(g.title.key, "diag.summary.gpu.title");
        assert_eq!(g.confidence, 81);
        assert_eq!(g.at_unix, 1_700_000_000);
        // Переживає перезапуск: JSON туди й назад без втрат.
        let back: LastGame = serde_json::from_slice(&serde_json::to_vec(&g).unwrap()).unwrap();
        assert_eq!(back, g);
        assert!(LastGame::from_result(&result(vec![]), 0).is_none());
        let mut short = r.clone();
        short.diagnosis.bottleneck = Bottleneck::InsufficientData;
        assert!(LastGame::from_result(&short, 0).is_none());
    }

    #[test]
    fn config_counts_problems_and_picks_driver() {
        let report = AuditReport {
            findings: vec![
                finding("audit", "ram.single_channel", Impact::High),
                finding("audit", "display.hz", Impact::Moderate),
                finding("audit", "driver.old", Impact::Moderate),
                finding("audit", "os.ok", Impact::Info),
            ],
            skipped: vec![],
        };
        let s = ConfigSummary::from_report(report);
        assert_eq!((s.high, s.moderate), (1, 2));
        assert_eq!(s.driver.map(|f| f.id), Some("driver.old".to_string()));
    }
}
