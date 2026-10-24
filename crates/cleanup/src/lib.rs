//! Очищення диска з вердиктами: що займає місце, чи можна це видаляти, що це і що буде після.
//!
//! Модуль не «чистить усе підряд», а пояснює. Видаляється **лише** те, що збіглося з правилом
//! бази (`rules/*.toml`); правило без джерела чи перевірки — ⚪ і не видаляє нічого. Жорсткий
//! чорний список ([`guard`]) правила перекрити не можуть. Перед будь-яким видаленням — сухий
//! прогін ([`plan::build_plan`]); 🟡 — лише через карантин; кожна дія — у журналі.
//!
//! Мережевих викликів немає: база правил оновлюється лише разом із програмою.
//! Усі файлові операції — через [`fs::FsApi`], тож логіку перевірено тестами на моку.

#![forbid(unsafe_code)]

pub mod fs;
pub mod guard;
pub mod journal;
pub mod path;
pub mod pattern;
pub mod plan;
pub mod quarantine;
pub mod rules;
pub mod scan;
pub mod snapshot;
pub mod system;

pub use fs::{FsApi, FsError, MemFs, StdFs};
pub use guard::{Guard, Zone};
pub use journal::{CleanupJournal, JournalLine, JournalRecord, Outcome, RunKind};
pub use pattern::Env;
pub use plan::{build_plan, execute, Action, CleanPlan, ExecContext, ExecProgress, ExecResult};
pub use quarantine::{Quarantine, QuarantineBatch};
pub use rules::{Category, Method, RuleIssue, RuleSet, Verdict};
pub use scan::{scan, Item, ScanOptions, ScanProgress, ScanReport};
pub use snapshot::{growth, pick_baseline, GrowthReport, Snapshot, SnapshotStore};
pub use system::SystemTools;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Помилки модуля очищення.
#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    /// Помилка файлової системи.
    #[error("{0}")]
    Fs(#[from] FsError),
    /// Об'єкт не можна видаляти (немає правила, чорний список, ⚪/🔴).
    #[error("не можна видаляти: {0}")]
    NotDeletable(String),
    /// Не знайдено.
    #[error("не знайдено: {0}")]
    NotFound(String),
    /// Штатний засіб Windows (DISM, powercfg) не запустився.
    #[error("штатний засіб: {0}")]
    Tool(String),
    /// Помилка JSON (журнал, карантин, знімки).
    #[error("помилка JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Скасування довгої операції (скан, очищення).
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Новий.
    pub fn new() -> Self {
        Self::default()
    }

    /// Просить зупинитись.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Чи попросили зупинитись.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
