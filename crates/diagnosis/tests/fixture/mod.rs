//! Регресійні записи: знеособлений запис гри (`<name>.json.gz`) і очікування (`<name>.toml`)
//! у `crates/diagnosis/tests/recordings/`. Спільне для тесту `recordings.rs` і прикладу
//! `fixture` (створення записів).

#![allow(dead_code)]

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use optimax_diagnosis::{Context, Diagnosis};
use optimax_frametime::Recording;
use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Системні процеси, імена яких лишаються в знеособленому записі: на них тримаються правила
/// (Defender) або вони однакові на всіх ПК з Windows.
pub const KEEP_PROCESSES: &[&str] = &[
    optimax_diagnosis::rules::DEFENDER_PROCESS,
    "System",
    "Registry",
    "Idle",
    "svchost.exe",
    "dwm.exe",
    "csrss.exe",
    "explorer.exe",
    "audiodg.exe",
    "lsass.exe",
    "services.exe",
    "wininit.exe",
    "SearchIndexer.exe",
    "TiWorker.exe",
    "TrustedInstaller.exe",
    "MoUsoCoreWorker.exe",
];

/// Опис запису й очікування (`<name>.toml`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    /// Що записано: гра, налаштування, сцена — людиною, для людей.
    pub description: String,
    /// Дата запису.
    pub recorded: String,
    /// Частота монітора під час запису (для розпізнавання V-Sync).
    pub display_hz: Option<u32>,
    /// Що має показати діагностика.
    pub expect: Expect,
}

/// Очікування. Порожнє поле — не перевіряється.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// Вузьке місце (`gpu`, `cpu`, `frame_cap`, `unclear`, `insufficient_data`).
    pub bottleneck: Option<String>,
    /// Впевненість головного вердикту не нижча, %.
    pub min_confidence: Option<u8>,
    /// Ці вердикти (id) мають бути.
    #[serde(default)]
    pub has: Vec<String>,
    /// Цих вердиктів (id) не має бути.
    #[serde(default)]
    pub absent: Vec<String>,
    /// Середній FPS у межах `[від, до]`.
    pub avg_fps: Option<[f64; 2]>,
    /// 1% low у межах `[від, до]`.
    pub low_1pct_fps: Option<[f64; 2]>,
    /// Плавність у розповіді (`smooth`, `isolated`, `noticeable`, `severe`).
    pub smoothness: Option<String>,
    /// Підсумок розповіді (`fine`, `fixable`, `unexplained`, `insufficient`).
    pub outcome: Option<String>,
}

/// Значення enum так, як його серіалізує serde (`noticeable`, `fine`…).
fn serde_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Тека з записами.
pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/recordings")
}

/// Імена всіх записів (за файлами `*.toml`), за абеткою.
pub fn names() -> std::io::Result<Vec<String>> {
    let mut out: Vec<String> = std::fs::read_dir(dir())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    out.sort();
    Ok(out)
}

fn recording_path(name: &str) -> PathBuf {
    dir().join(format!("{name}.json.gz"))
}

/// Читає запис і очікування.
pub fn load(name: &str) -> anyhow::Result<(Recording, Fixture)> {
    let toml_text = std::fs::read_to_string(dir().join(format!("{name}.toml")))?;
    let fixture: Fixture = toml::from_str(&toml_text)?;
    let rec = read_recording(&recording_path(name))?;
    Ok((rec, fixture))
}

/// Читає стиснений запис.
pub fn read_recording(path: &Path) -> anyhow::Result<Recording> {
    let mut json = String::new();
    GzDecoder::new(std::fs::File::open(path)?).read_to_string(&mut json)?;
    Ok(serde_json::from_str(&json)?)
}

/// Пише стиснений запис у теку записів.
pub fn write_recording(name: &str, rec: &Recording) -> anyhow::Result<PathBuf> {
    let path = recording_path(name);
    write_recording_at(&path, rec)?;
    Ok(path)
}

/// Пише стиснений запис за шляхом.
pub fn write_recording_at(path: &Path, rec: &Recording) -> anyhow::Result<()> {
    let mut gz = GzEncoder::new(std::fs::File::create(path)?, Compression::best());
    gz.write_all(serde_json::to_string(rec)?.as_bytes())?;
    gz.finish()?;
    Ok(())
}

/// Контекст аналізу для запису: лише частота монітора. Аудит конфігурації й кеш шейдерів
/// залежать від ПК, тож у регресійних тестах їх немає.
pub fn context(f: &Fixture) -> Context {
    Context {
        display_hz: f.display_hz,
        ..Context::default()
    }
}

/// Рядок вузького місця так, як його серіалізує serde (`gpu`, `frame_cap`…).
pub fn bottleneck_name(d: &Diagnosis) -> String {
    serde_json::to_value(d.bottleneck)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Що не збіглося з очікуваннями (порожньо — усе гаразд).
pub fn check(d: &Diagnosis, e: &Expect) -> Vec<String> {
    let mut fails = Vec::new();
    let ids: Vec<&str> = d.verdicts.iter().map(|v| v.finding.id.as_str()).collect();
    let got = bottleneck_name(d);
    if let Some(b) = &e.bottleneck {
        if *b != got {
            fails.push(format!("вузьке місце: очікувалось {b}, вийшло {got}"));
        }
    }
    if let (Some(min), Some(head)) = (e.min_confidence, d.verdicts.first()) {
        let c = head.finding.confidence.percent();
        if c < min {
            fails.push(format!("впевненість {c} % < {min} %"));
        }
    }
    for id in &e.has {
        if !ids.contains(&id.as_str()) {
            fails.push(format!("немає вердикту {id}"));
        }
    }
    for id in &e.absent {
        if ids.contains(&id.as_str()) {
            fails.push(format!("зайвий вердикт {id}"));
        }
    }
    let range = |name: &str, v: Option<f64>, r: Option<[f64; 2]>, fails: &mut Vec<String>| {
        if let Some([lo, hi]) = r {
            match v {
                Some(v) if v >= lo && v <= hi => {}
                Some(v) => fails.push(format!("{name} {v:.1} поза [{lo}, {hi}]")),
                None => fails.push(format!("{name}: немає статистики")),
            }
        }
    };
    let stats = d.stats.as_ref();
    range(
        "середній FPS",
        stats.map(|s| s.avg_fps),
        e.avg_fps,
        &mut fails,
    );
    range(
        "1% low",
        stats.map(|s| s.low_1pct_fps),
        e.low_1pct_fps,
        &mut fails,
    );
    if e.smoothness.is_some() || e.outcome.is_some() {
        match &d.story {
            None => fails.push("немає розповіді".into()),
            Some(s) => {
                let pairs = [
                    ("плавність", &e.smoothness, serde_name(&s.smoothness)),
                    ("підсумок розповіді", &e.outcome, serde_name(&s.outcome)),
                ];
                for (name, want, got) in pairs {
                    if let Some(want) = want {
                        if *want != got {
                            fails.push(format!("{name}: очікувалось {want}, вийшло {got}"));
                        }
                    }
                }
            }
        }
    }
    if !fails.is_empty() {
        fails.push(format!("усі вердикти: {}", ids.join(", ")));
    }
    fails
}
