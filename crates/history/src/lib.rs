//! Історія записів гри («Діагностика»): підсумок кожного запису, знімок стану ПК на момент
//! запису, порівняння двох записів і що змінилося між ними (драйвер, Windows, файл гри…).
//!
//! Лише чисті функції над даними; файли — через трейт [`Files`] (у тестах — у пам'яті).
//! Повні записи (кадри й метрики) крейт зберігає як непрозорі байти: стискає й розбирає їх
//! застосунок.

#![forbid(unsafe_code)]

mod compare;
mod model;
mod session;
mod store;

pub use compare::{
    compare, diff, driver_effects, fps_noise_pct, regressions, DriverEffect, RecordComparison,
    Regression, Trend, MIN_SIGNIFICANT_PCT,
};
pub use model::{
    clean_scene, scene_match, DisplayState, FileStamp, HistoryEntry, SceneMatch, StateChange,
    StateField, SystemState,
};
pub use session::{
    cause_summary, keep_worst, FreezeCauseCount, FreezeEntry, SessionEntry, SessionStore,
    KEEP_FREEZE_RECORDINGS, KEEP_SESSIONS, UNKNOWN_CAUSE,
};
pub use store::{entry_id, Files, HistoryError, MemFiles, StdDir, Store, KEEP_RECORDINGS_PER_GAME};
