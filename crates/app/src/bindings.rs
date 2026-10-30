//! Генерація TypeScript-типів для інтерфейсу: `cargo test -p optimax-app`.
//!
//! Файли пишуться в `ui/src/lib/bindings/` разом з усіма залежними типами. 64-бітні цілі
//! віддаються як `number` (serde_json серіалізує їх числами; ідентифікатори процесів у UI
//! назад не надсилаються, тож втрата точності понад 2^53 неможлива).

use crate::dto::*;
use crate::error::CmdError;
use crate::log_layer::LogLine;
use crate::settings::Settings;
use std::path::PathBuf;
use ts_rs::{Config, TS};

fn out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/lib/bindings")
}

/// Вміст без різниці в кінцях рядків (у робочій копії Windows вони можуть бути CRLF).
fn normalized(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.replace("\r\n", "\n"))
}

/// Переносить згенероване з `tmp` у `dir`: пише лише змінені файли й видаляє застарілі,
/// щоб не «змінювати» в git файли з тим самим вмістом.
fn sync_dir(tmp: &std::path::Path, dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let fresh: Vec<_> = std::fs::read_dir(tmp)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    for src in &fresh {
        let Some(name) = src.file_name() else {
            continue;
        };
        let dst = dir.join(name);
        if normalized(src) != normalized(&dst) {
            std::fs::copy(src, &dst)?;
        }
    }
    for old in std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())) {
        if !fresh.iter().any(|f| f.file_name() == old.file_name()) {
            std::fs::remove_file(old)?;
        }
    }
    Ok(())
}

#[test]
fn export_typescript_bindings() {
    let dir = out_dir();
    let tmp = std::env::temp_dir().join(format!("optimax-bindings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let cfg = Config::new().with_large_int("number").with_out_dir(&tmp);
    macro_rules! export {
        ($($t:ty),* $(,)?) => {$(
            <$t>::export_all(&cfg).unwrap_or_else(|e| panic!("{}: {e}", stringify!($t)));
        )*};
    }
    export!(
        AppInfo,
        StartupReport,
        Settings,
        CmdError,
        LogLine,
        optimax_audit::AuditReport,
        HardwareInfo,
        optimax_metrics::Snapshot,
        ProcessEntry,
        optimax_frametime::RecordProgress,
        DiagnosisResult,
        PowerSchemes,
        UiDesired,
        StartupEntry,
        PlanHandle,
        ApplyResult,
        optimax_tweaks::JournalEntry,
        RollbackResult,
        AbView,
        AbFinal,
        optimax_gamemode::GameProfile,
        crate::commands::upscale::UpscaleView,
        crate::commands::network::NetView,
        crate::commands::network::NetProgress,
        crate::commands::network::NetResult,
        crate::commands::laptop::LaptopView,
        crate::commands::cleanup::DriveView,
        crate::commands::cleanup::ScanView,
        crate::commands::cleanup::DryRunView,
        crate::commands::cleanup::ExecView,
        crate::commands::cleanup::QuarantineView,
        crate::commands::cleanup::GrowthView,
        optimax_cleanup::ScanProgress,
        optimax_cleanup::ExecProgress,
        optimax_cleanup::JournalRecord,
        ProfilePreview,
        GameModeEvent,
    );
    sync_dir(&tmp, &dir).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(dir.join("Finding.ts").is_file());
    assert!(dir.join("Text.ts").is_file());
}
