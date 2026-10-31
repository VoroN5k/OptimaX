//! Журнал очищення: кожна дія — час, правило, шлях, розмір, результат, сума звільненого.
//! Запис створюється **до** початку дії (стан «триває») й доповнюється після, тож навіть після
//! збою видно, що відбувалося. Зберігається локально (`cleanup-journal.json` у захищеній теці даних програми).

use crate::fs::FsApi;
use crate::CleanupError;
use serde::{Deserialize, Serialize};

/// Скільки записів зберігати.
pub const MAX_RECORDS: usize = 200;
/// Скільки рядків-файлів зберігати в одному записі (решта — лише лічильником).
pub const MAX_LINES: usize = 2000;

/// Вид дії.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// Очищення (видалення 🟢 і карантин 🟡).
    Clean,
    /// Відновлення з карантину.
    Restore,
    /// Остаточне видалення з карантину.
    Purge,
    /// Штатне очищення компонентів (DISM).
    Dism,
    /// Вимкнення гібернації.
    HibernateOff,
    /// Увімкнення гібернації (повернення).
    HibernateOn,
}

/// Результат для одного файлу.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "CleanupOutcome"))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// Видалено.
    Deleted,
    /// Перенесено в карантин.
    Quarantined,
    /// Відновлено з карантину.
    Restored,
    /// Видалено з карантину остаточно.
    Purged,
    /// Пропущено.
    Skipped {
        /// Причина.
        reason: String,
    },
}

/// Рядок журналу.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JournalLine {
    /// Шлях.
    pub path: String,
    /// Правило.
    pub rule_id: Option<String>,
    /// Розмір.
    pub bytes: u64,
    /// Результат.
    pub outcome: Outcome,
}

/// Запис журналу.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JournalRecord {
    /// Номер.
    pub id: u64,
    /// Вид.
    pub kind: RunKind,
    /// Початок (Unix-секунди).
    pub started_unix: i64,
    /// Кінець (`None` — дія не завершилась: програма впала посередині).
    pub finished_unix: Option<i64>,
    /// Звільнено байтів (виміряно: сума видалених файлів або різниця вільного місця для DISM).
    pub freed_bytes: u64,
    /// Перенесено в карантин байтів.
    pub quarantined_bytes: u64,
    /// Відновлено байтів.
    pub restored_bytes: u64,
    /// Пропущено файлів.
    pub skipped: u64,
    /// Партія карантину.
    pub batch_id: Option<String>,
    /// Примітка (служби, вихід DISM тощо).
    pub note: Option<String>,
    /// Рядки (не більше [`MAX_LINES`]).
    pub lines: Vec<JournalLine>,
    /// Усього рядків (включно з невключеними).
    pub lines_total: u64,
}

/// Журнал у JSON-файлі.
pub struct CleanupJournal<'a> {
    fs: &'a dyn FsApi,
    path: String,
}

impl<'a> CleanupJournal<'a> {
    /// Журнал за шляхом.
    pub fn new(fs: &'a dyn FsApi, path: &str) -> Self {
        Self {
            fs,
            path: path.to_string(),
        }
    }

    /// Усі записи (найновіші — останні).
    pub fn load(&self) -> Result<Vec<JournalRecord>, CleanupError> {
        match self.fs.read_text(&self.path).map_err(CleanupError::Fs)? {
            Some(t) if !t.trim().is_empty() => Ok(serde_json::from_str(&t)?),
            _ => Ok(Vec::new()),
        }
    }

    fn save(&self, records: &[JournalRecord]) -> Result<(), CleanupError> {
        let start = records.len().saturating_sub(MAX_RECORDS);
        let json = serde_json::to_string_pretty(&records[start..])?;
        self.fs
            .write_text_atomic(&self.path, &json)
            .map_err(CleanupError::Fs)
    }

    /// Створює запис «триває» перед дією. Повертає його номер.
    pub fn begin(&self, kind: RunKind, now_unix: i64) -> Result<u64, CleanupError> {
        let mut records = self.load()?;
        let id = records.last().map_or(1, |r| r.id + 1);
        records.push(JournalRecord {
            id,
            kind,
            started_unix: now_unix,
            finished_unix: None,
            freed_bytes: 0,
            quarantined_bytes: 0,
            restored_bytes: 0,
            skipped: 0,
            batch_id: None,
            note: None,
            lines: Vec::new(),
            lines_total: 0,
        });
        self.save(&records)?;
        Ok(id)
    }

    /// Завершує запис: `fill` заповнює підсумки, рядки обрізаються до [`MAX_LINES`].
    pub fn finish(
        &self,
        id: u64,
        now_unix: i64,
        lines: Vec<JournalLine>,
        fill: impl FnOnce(&mut JournalRecord),
    ) -> Result<JournalRecord, CleanupError> {
        let mut records = self.load()?;
        let rec = records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| CleanupError::NotFound(format!("запис журналу #{id}")))?;
        rec.finished_unix = Some(now_unix);
        rec.lines_total = lines.len() as u64;
        rec.skipped = lines
            .iter()
            .filter(|l| matches!(l.outcome, Outcome::Skipped { .. }))
            .count() as u64;
        // Пропущені — першими: саме їх користувач захоче побачити.
        let mut lines = lines;
        lines.sort_by_key(|l| !matches!(l.outcome, Outcome::Skipped { .. }));
        lines.truncate(MAX_LINES);
        rec.lines = lines;
        fill(rec);
        let out = rec.clone();
        self.save(&records)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::MemFs;

    #[test]
    fn begin_then_finish() {
        let fs = MemFs::new();
        let j = CleanupJournal::new(&fs, r"C:\data\journal.json");
        let id = j.begin(RunKind::Clean, 100).unwrap();
        assert_eq!(
            j.load().unwrap()[0].finished_unix,
            None,
            "видно незавершену дію"
        );
        let lines = vec![
            JournalLine {
                path: r"C:\t\a".into(),
                rule_id: Some("user_temp".into()),
                bytes: 10,
                outcome: Outcome::Deleted,
            },
            JournalLine {
                path: r"C:\t\b".into(),
                rule_id: Some("user_temp".into()),
                bytes: 5,
                outcome: Outcome::Skipped {
                    reason: "зайнятий".into(),
                },
            },
        ];
        let rec = j.finish(id, 200, lines, |r| r.freed_bytes = 10).unwrap();
        assert_eq!(rec.skipped, 1);
        assert_eq!(rec.lines[0].path, r"C:\t\b");
        let loaded = j.load().unwrap();
        assert_eq!(loaded[0].finished_unix, Some(200));
        assert_eq!(loaded[0].freed_bytes, 10);
        assert_eq!(j.begin(RunKind::Dism, 300).unwrap(), 2);
    }
}
