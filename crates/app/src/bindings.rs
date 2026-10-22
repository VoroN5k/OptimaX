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

#[test]
fn export_typescript_bindings() {
    let dir = out_dir();
    // Перегенеровуємо з нуля, щоб не лишалося файлів від видалених типів.
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = Config::new().with_large_int("number").with_out_dir(&dir);
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
        RecordingProgress,
        DiagnosisResult,
        PowerSchemes,
        UiDesired,
        PlanHandle,
        ApplyResult,
        optimax_tweaks::JournalEntry,
        RollbackResult,
        AbView,
        AbFinal,
        optimax_gamemode::GameProfile,
        ProfilePreview,
        GameModeEvent,
    );
    assert!(dir.join("Finding.ts").is_file());
    assert!(dir.join("Text.ts").is_file());
}
