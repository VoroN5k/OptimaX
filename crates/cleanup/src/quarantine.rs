//! Карантин для 🟡: файли переносяться в окрему теку й можуть бути відновлені впродовж N днів.
//!
//! Чесність щодо місця: карантин на тому самому диску **не звільняє** місце (файл лише
//! перейменовується). Тоді місце звільниться після закінчення карантину; на іншому диску —
//! одразу (файл копіюється, оригінал видаляється).
//!
//! Метадані — JSON-індекс поза текою файлів (`%LOCALAPPDATA%\OptimaX\quarantine.json`).

use crate::fs::{FsApi, FsError, FsErrorKind, Kind};
use crate::journal::{JournalLine, Outcome};
use crate::path;
use crate::rules::RuleSet;
use crate::CleanupError;
use serde::{Deserialize, Serialize};

/// Файл у карантині.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QuarantineEntry {
    /// Початковий шлях.
    pub original: String,
    /// Де лежить зараз.
    pub stored: String,
    /// Розмір.
    pub bytes: u64,
    /// Правило.
    pub rule_id: String,
}

/// Стан партії.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum BatchState {
    /// Файли в карантині.
    Active,
    /// Відновлено.
    Restored,
    /// Видалено остаточно.
    Purged,
}

/// Партія карантину: одне очищення.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QuarantineBatch {
    /// Ідентифікатор.
    pub id: String,
    /// Створено (Unix-секунди).
    pub created_unix: i64,
    /// Кінець карантину (Unix-секунди).
    pub expires_unix: i64,
    /// Тека партії.
    pub dir: String,
    /// На тому самому диску, що й оригінали: місце звільниться лише після закінчення карантину.
    pub same_volume: bool,
    /// Стан.
    pub state: BatchState,
    /// Файли.
    pub entries: Vec<QuarantineEntry>,
}

impl QuarantineBatch {
    /// Сума байтів.
    pub fn bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.bytes).sum()
    }
}

/// Партія, що наповнюється під час очищення.
#[derive(Debug)]
pub struct BatchWriter {
    batch: QuarantineBatch,
    created_dir: bool,
}

/// Результат відновлення / остаточного видалення.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatchOutcome {
    /// Оброблено байтів.
    pub bytes: u64,
    /// Рядки журналу.
    pub lines: Vec<JournalLine>,
}

/// Карантин.
pub struct Quarantine<'a> {
    fs: &'a dyn FsApi,
    index: String,
    root: String,
    days: u32,
    batches: Vec<QuarantineBatch>,
}

fn fs_err(e: FsError) -> CleanupError {
    CleanupError::Fs(e)
}

/// Назва теки карантину (останній компонент кореня).
pub const QUARANTINE_DIR_NAME: &str = "Quarantine";

/// Файл лежить у теці своєї партії (`…\Quarantine\<id>\N`). Перевіряється за текою партії, а не
/// поточним коренем: після зміни диска карантину старі партії лишаються відновлюваними.
fn stored_in(batch_dir: &str, stored: &str) -> bool {
    let root_ok = path::parent(batch_dir)
        .is_some_and(|r| path::file_name(r).eq_ignore_ascii_case(QUARANTINE_DIR_NAME));
    root_ok
        && path::normalize(stored).is_ok_and(|s| path::eq_ci(&s, stored))
        && path::parent(stored).is_some_and(|p| path::eq_ci(p, batch_dir))
}

fn same_drive(a: &str, b: &str) -> bool {
    path::drive_letter(a).map(|c| c.to_ascii_uppercase())
        == path::drive_letter(b).map(|c| c.to_ascii_uppercase())
}

impl<'a> Quarantine<'a> {
    /// Відкриває карантин: `index` — JSON-файл метаданих, `root` — тека для файлів.
    pub fn open(
        fs: &'a dyn FsApi,
        index: &str,
        root: &str,
        days: u32,
    ) -> Result<Self, CleanupError> {
        let root = path::normalize(root).map_err(|e| CleanupError::NotDeletable(e.to_string()))?;
        if !path::file_name(&root).eq_ignore_ascii_case(QUARANTINE_DIR_NAME) {
            return Err(CleanupError::NotDeletable(format!(
                "тека карантину має називатися {QUARANTINE_DIR_NAME}: {root}"
            )));
        }
        let batches = match fs.read_text(index).map_err(fs_err)? {
            Some(t) if !t.trim().is_empty() => serde_json::from_str(&t)?,
            _ => Vec::new(),
        };
        Ok(Self {
            fs,
            index: index.to_string(),
            root,
            days: days.max(1),
            batches,
        })
    }

    /// Усі партії (найновіші — останні).
    pub fn batches(&self) -> &[QuarantineBatch] {
        &self.batches
    }

    /// Тека для файлів.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Чи на тому самому диску, що й шлях.
    pub fn same_volume_as(&self, p: &str) -> bool {
        path::drive_letter(p).map(|c| c.to_ascii_uppercase())
            == path::drive_letter(&self.root).map(|c| c.to_ascii_uppercase())
    }

    fn save(&self) -> Result<(), CleanupError> {
        let json = serde_json::to_string_pretty(&self.batches)?;
        self.fs
            .write_text_atomic(&self.index, &json)
            .map_err(fs_err)
    }

    /// Починає партію.
    pub fn begin_batch(&mut self, now_unix: i64) -> BatchWriter {
        let mut n = 0;
        let id = loop {
            let id = if n == 0 {
                format!("{now_unix}")
            } else {
                format!("{now_unix}-{n}")
            };
            if !self.batches.iter().any(|b| b.id == id) {
                break id;
            }
            n += 1;
        };
        BatchWriter {
            batch: QuarantineBatch {
                dir: path::join(&self.root, &id),
                id,
                created_unix: now_unix,
                expires_unix: now_unix + i64::from(self.days) * 86_400,
                same_volume: true,
                state: BatchState::Active,
                entries: Vec::new(),
            },
            created_dir: false,
        }
    }

    /// Переносить файл у партію. Між томами — копія, перевірка розміру, потім видалення оригіналу.
    pub fn put(
        &mut self,
        w: &mut BatchWriter,
        file: &str,
        rule_id: &str,
        bytes: u64,
    ) -> Result<(), FsError> {
        if !w.created_dir {
            self.fs.create_dir_all(&w.batch.dir)?;
            w.created_dir = true;
        }
        let stored = path::join(&w.batch.dir, &w.batch.entries.len().to_string());
        let same = self.same_volume_as(file);
        if same {
            self.fs.rename(file, &stored)?;
        } else {
            let copied = self.fs.copy_file(file, &stored)?;
            if copied != bytes {
                let _ = self.fs.remove_file(&stored);
                return Err(FsError::new(FsErrorKind::Other, "розмір копії не збігся"));
            }
            if let Err(e) = self.fs.remove_file(file) {
                // Оригінал зайнятий — копію прибираємо, файл лишається на місці.
                let _ = self.fs.remove_file(&stored);
                return Err(e);
            }
            w.batch.same_volume = false;
        }
        w.batch.entries.push(QuarantineEntry {
            original: file.to_string(),
            stored,
            bytes,
            rule_id: rule_id.to_string(),
        });
        // Індекс оновлюється одразу: якщо програма впаде посеред очищення, файли не загубляться.
        self.save_with(&w.batch)
            .map_err(|e| FsError::new(FsErrorKind::Other, e.to_string()))
    }

    fn save_with(&self, batch: &QuarantineBatch) -> Result<(), CleanupError> {
        let mut all = self.batches.clone();
        all.push(batch.clone());
        let json = serde_json::to_string_pretty(&all)?;
        self.fs
            .write_text_atomic(&self.index, &json)
            .map_err(fs_err)
    }

    /// Завершує партію. Порожня партія не зберігається.
    pub fn finish_batch(&mut self, w: BatchWriter) -> Result<Option<String>, CleanupError> {
        if w.batch.entries.is_empty() {
            if w.created_dir {
                let _ = self.fs.remove_empty_dir(&w.batch.dir);
            }
            return Ok(None);
        }
        let id = w.batch.id.clone();
        self.batches.push(w.batch);
        self.save()?;
        Ok(Some(id))
    }

    fn active_mut(&mut self, id: &str) -> Result<usize, CleanupError> {
        self.batches
            .iter()
            .position(|b| b.id == id && b.state == BatchState::Active)
            .ok_or_else(|| CleanupError::NotFound(format!("партію карантину {id}")))
    }

    /// Повертає файли партії на місце. Файл, на місці якого вже щось є, лишається в карантині.
    /// Відновлення можливе лише в межах кореня правила й поза чорним списком.
    pub fn restore(&mut self, id: &str, rules: &RuleSet) -> Result<BatchOutcome, CleanupError> {
        let idx = self.active_mut(id)?;
        let batch = self.batches[idx].clone();
        let mut out = BatchOutcome::default();
        let mut left = Vec::new();
        for e in batch.entries {
            let result = self.restore_one(&batch.dir, &e, rules);
            let outcome = match result {
                Ok(()) => {
                    out.bytes += e.bytes;
                    Outcome::Restored
                }
                Err(reason) => {
                    left.push(e.clone());
                    Outcome::Skipped { reason }
                }
            };
            out.lines.push(JournalLine {
                path: e.original.clone(),
                rule_id: Some(e.rule_id.clone()),
                bytes: e.bytes,
                outcome,
            });
        }
        let b = &mut self.batches[idx];
        if left.is_empty() {
            b.state = BatchState::Restored;
            let _ = self.fs.remove_empty_dir(&b.dir);
        }
        b.entries = left;
        self.save()?;
        Ok(out)
    }

    fn restore_one(&self, dir: &str, e: &QuarantineEntry, rules: &RuleSet) -> Result<(), String> {
        let original = path::normalize(&e.original).map_err(|x| x.to_string())?;
        let cls = rules.classify(&original);
        let rule_ok = cls
            .rule
            .and_then(|i| rules.get(i))
            .is_some_and(|r| r.rule.id == e.rule_id);
        if !rule_ok || !rules.guard().allows_delete(&original) {
            return Err("шлях більше не відповідає правилу".into());
        }
        if !stored_in(dir, &e.stored) {
            return Err("файл карантину поза текою карантину".into());
        }
        if self.fs.metadata(&original).is_ok() {
            return Err("на місці вже є файл".into());
        }
        let parent = path::parent(&original).ok_or("немає батьківського каталогу")?;
        self.fs.create_dir_all(parent).map_err(|x| x.to_string())?;
        if !self
            .fs
            .canonicalize(parent)
            .is_ok_and(|c| path::eq_ci(&c, parent))
        {
            return Err("шлях перенаправлено (junction/symlink)".into());
        }
        if same_drive(&e.stored, &original) {
            self.fs
                .rename(&e.stored, &original)
                .map_err(|x| x.to_string())
        } else {
            self.fs
                .copy_file(&e.stored, &original)
                .map_err(|x| x.to_string())?;
            self.fs.remove_file(&e.stored).map_err(|x| x.to_string())
        }
    }

    /// Видаляє файли партії остаточно.
    pub fn purge(&mut self, id: &str) -> Result<BatchOutcome, CleanupError> {
        let idx = self.active_mut(id)?;
        let batch = self.batches[idx].clone();
        let mut out = BatchOutcome::default();
        let mut left = Vec::new();
        for e in batch.entries {
            let inside = stored_in(&batch.dir, &e.stored);
            let plain = self
                .fs
                .metadata(&e.stored)
                .is_ok_and(|m| m.kind == Kind::File && !m.reparse);
            let result = if !inside || !plain {
                Err("не файл у теці карантину".to_string())
            } else {
                self.fs.remove_file(&e.stored).map_err(|x| x.to_string())
            };
            let outcome = match result {
                Ok(()) => {
                    out.bytes += e.bytes;
                    Outcome::Purged
                }
                Err(reason) => {
                    left.push(e.clone());
                    Outcome::Skipped { reason }
                }
            };
            out.lines.push(JournalLine {
                path: e.original.clone(),
                rule_id: Some(e.rule_id.clone()),
                bytes: e.bytes,
                outcome,
            });
        }
        let b = &mut self.batches[idx];
        if left.is_empty() {
            b.state = BatchState::Purged;
            let _ = self.fs.remove_empty_dir(&b.dir);
        }
        b.entries = left;
        self.save()?;
        Ok(out)
    }

    /// Партії, термін яких минув.
    pub fn expired(&self, now_unix: i64) -> Vec<String> {
        self.batches
            .iter()
            .filter(|b| b.state == BatchState::Active && b.expires_unix <= now_unix)
            .map(|b| b.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::MemFs;
    use crate::pattern::Env;
    use crate::plan::{build_plan, execute, Action, ExecContext};
    use crate::CancelToken;

    const NOW: i64 = 1_800_000_000;
    const DX: &str = r"C:\Users\Test\AppData\Local\NVIDIA\DXCache";
    const INDEX: &str = r"C:\Users\Test\AppData\Local\OptimaX\quarantine.json";

    fn setup() -> (MemFs, RuleSet) {
        let fs = MemFs::new();
        fs.add_file(&format!(r"{DX}\a.nvph"), 400, 0).add_file(
            &format!(r"{DX}\sub\b.nvph"),
            100,
            0,
        );
        (fs, RuleSet::embedded(Env::for_tests()).0)
    }

    fn quarantine_run(fs: &MemFs, rs: &RuleSet, q: &mut Quarantine<'_>) -> crate::plan::ExecResult {
        let cancel = CancelToken::new();
        let plan = build_plan(fs, rs, &[DX.to_string()], NOW, &cancel).unwrap();
        assert_eq!(plan.items[0].action, Action::Quarantine);
        let ctx = ExecContext {
            fs,
            rules: rs,
            services: None,
            now_unix: NOW,
            cancel: &cancel,
        };
        execute(&ctx, &plan, Some(q), &|_| {}).unwrap()
    }

    #[test]
    fn move_and_restore_same_volume() {
        let (fs, rs) = setup();
        let qroot = r"C:\Users\Test\AppData\Local\OptimaX\Quarantine";
        let mut q = Quarantine::open(&fs, INDEX, qroot, 7).unwrap();
        let res = quarantine_run(&fs, &rs, &mut q);
        assert_eq!(res.quarantined_bytes, 500);
        assert_eq!(
            res.freed_bytes, 0,
            "карантин на тому ж диску місця не звільняє"
        );
        assert!(!fs.exists(&format!(r"{DX}\a.nvph")));
        let batch = q.batches()[0].clone();
        assert!(batch.same_volume);
        assert_eq!(batch.expires_unix, NOW + 7 * 86_400);
        // Метадані на диску збігаються з тим, що в пам'яті.
        let reopened = Quarantine::open(&fs, INDEX, qroot, 7).unwrap();
        assert_eq!(reopened.batches(), q.batches());
        let mut originals: Vec<_> = batch.entries.iter().map(|e| e.original.clone()).collect();
        originals.sort();
        assert_eq!(
            originals,
            vec![format!(r"{DX}\a.nvph"), format!(r"{DX}\sub\b.nvph")]
        );
        // Відновлення.
        let out = q.restore(&batch.id, &rs).unwrap();
        assert_eq!(out.bytes, 500);
        assert_eq!(fs.len_of(&format!(r"{DX}\a.nvph")), Some(400));
        assert_eq!(fs.len_of(&format!(r"{DX}\sub\b.nvph")), Some(100));
        assert_eq!(q.batches()[0].state, BatchState::Restored);
    }

    #[test]
    fn other_volume_copies_and_frees() {
        let (fs, rs) = setup();
        fs.add_dir(r"D:\");
        let mut q = Quarantine::open(&fs, INDEX, r"D:\OptimaX\Quarantine", 7).unwrap();
        let res = quarantine_run(&fs, &rs, &mut q);
        assert_eq!(res.quarantined_bytes, 500);
        let batch = q.batches()[0].clone();
        assert!(!batch.same_volume);
        assert!(batch.entries.iter().all(|e| e.stored.starts_with(r"D:\")));
        assert!(!fs.exists(&format!(r"{DX}\a.nvph")));
        // Користувач повернув карантин на системний диск — стара партія лишається відновлюваною.
        let mut q = Quarantine::open(
            &fs,
            INDEX,
            r"C:\Users\Test\AppData\Local\OptimaX\Quarantine",
            7,
        )
        .unwrap();
        q.restore(&batch.id, &rs).unwrap();
        assert_eq!(fs.len_of(&format!(r"{DX}\a.nvph")), Some(400));
        assert!(Quarantine::open(&fs, INDEX, r"C:\Users\Test\AppData\Local", 7).is_err());
    }

    #[test]
    fn restore_does_not_overwrite_and_purge_expires() {
        let (fs, rs) = setup();
        let qroot = r"C:\Users\Test\AppData\Local\OptimaX\Quarantine";
        let mut q = Quarantine::open(&fs, INDEX, qroot, 7).unwrap();
        quarantine_run(&fs, &rs, &mut q);
        let id = q.batches()[0].id.clone();
        // Драйвер уже створив новий файл з тим самим ім'ям.
        fs.add_file(&format!(r"{DX}\a.nvph"), 1, NOW);
        let out = q.restore(&id, &rs).unwrap();
        assert_eq!(out.bytes, 100);
        assert_eq!(
            fs.len_of(&format!(r"{DX}\a.nvph")),
            Some(1),
            "не перезаписано"
        );
        assert_eq!(q.batches()[0].state, BatchState::Active);
        assert!(q.expired(NOW).is_empty());
        assert_eq!(q.expired(NOW + 8 * 86_400), vec![id.clone()]);
        let out = q.purge(&id).unwrap();
        assert_eq!(out.bytes, 400);
        assert_eq!(q.batches()[0].state, BatchState::Purged);
    }

    #[test]
    fn locked_file_stays_in_place() {
        let (fs, rs) = setup();
        fs.lock_file(&format!(r"{DX}\a.nvph"));
        let mut q = Quarantine::open(
            &fs,
            INDEX,
            r"C:\Users\Test\AppData\Local\OptimaX\Quarantine",
            7,
        )
        .unwrap();
        let res = quarantine_run(&fs, &rs, &mut q);
        assert_eq!(res.quarantined_bytes, 100);
        assert!(fs.exists(&format!(r"{DX}\a.nvph")));
        assert_eq!(q.batches()[0].entries.len(), 1);
    }
}
