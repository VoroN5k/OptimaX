//! Шар `tracing`, що пересилає дії OptimaX (рівень INFO і вище) у журнал дій інтерфейсу.

use serde::Serialize;
use std::sync::mpsc::Sender;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Рядок журналу дій.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LogLine {
    /// `info` / `warn` / `error`.
    pub level: String,
    /// Модуль-джерело.
    pub target: String,
    /// Текст.
    pub message: String,
    /// Час, мс від Unix-епохи.
    pub unix_ms: u64,
}

/// Шар, що надсилає рядки в канал (далі їх отримує потік, який емітить події у вікно).
pub struct UiLogLayer {
    tx: Sender<LogLine>,
}

impl UiLogLayer {
    /// Новий шар.
    pub fn new(tx: Sender<LogLine>) -> Self {
        Self { tx }
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        }
    }
}

impl<S: Subscriber> Layer<S> for UiLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() > Level::INFO || !meta.target().starts_with("optimax") {
            return;
        }
        let mut v = MessageVisitor::default();
        event.record(&mut v);
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        let _ = self.tx.send(LogLine {
            level: meta.level().as_str().to_ascii_lowercase(),
            target: meta.target().to_string(),
            message: v.0,
            unix_ms,
        });
    }
}
