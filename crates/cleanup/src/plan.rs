//! Сухий прогін (план) і виконання очищення.
//!
//! План будується лише для коренів правил, вибраних зі звіту скану, і містить точний список файлів.
//! Виконання видаляє (🟢) або переносить у карантин (🟡) **лише файли з плану**, перевіряючи кожен
//! заново безпосередньо перед операцією: нормалізація, межі кореня, чорний список, правило,
//! канонічний шлях батьківського каталогу (junction / 8.3), reparse point.

use crate::fs::{FsApi, FsErrorKind, Kind};
use crate::journal::{JournalLine, Outcome};
use crate::path;
use crate::quarantine::Quarantine;
use crate::rules::{RuleSet, Verdict};
use crate::scan::{expand_roots, walk_root, Candidate};
use crate::{CancelToken, CleanupError};
use optimax_core::service::{ServiceApi, ServiceState};
use serde::Serialize;
use std::collections::HashMap;

/// Що зробити з файлами об'єкта.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Видалити (🟢).
    Delete,
    /// Перенести в карантин (🟡).
    Quarantine,
}

/// Чому файл пропущено.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum SkipReason {
    /// Шлях не пройшов нормалізацію.
    BadPath(String),
    /// Поза коренем правила.
    OutsideRoot,
    /// У чорному списку.
    Blacklisted,
    /// Шлях підпадає під суворіше правило або вже не під це.
    RuleMismatch,
    /// Шлях веде не туди, куди виглядає (junction, symlink, коротке ім'я 8.3 по дорозі).
    Redirected,
    /// Сам об'єкт — reparse point.
    Reparse,
    /// Файл зайнятий іншою програмою.
    Busy,
    /// Немає доступу.
    Denied,
    /// Файлу вже немає.
    Gone,
    /// Файл змінився після сухого прогону (тип чи розмір).
    Changed,
    /// Скасовано користувачем.
    Cancelled,
    /// Інша помилка.
    Error(String),
}

impl SkipReason {
    /// Короткий опис для журналу.
    pub fn describe(&self) -> String {
        match self {
            Self::BadPath(e) => format!("недопустимий шлях: {e}"),
            Self::OutsideRoot => "поза коренем правила".into(),
            Self::Blacklisted => "у чорному списку".into(),
            Self::RuleMismatch => "не відповідає правилу".into(),
            Self::Redirected => "шлях перенаправлено (junction/symlink/8.3)".into(),
            Self::Reparse => "reparse point".into(),
            Self::Busy => "зайнятий іншою програмою".into(),
            Self::Denied => "немає доступу".into(),
            Self::Gone => "вже немає".into(),
            Self::Changed => "змінився після сухого прогону".into(),
            Self::Cancelled => "скасовано".into(),
            Self::Error(e) => e.clone(),
        }
    }

    fn from_fs(kind: FsErrorKind, message: &str) -> Self {
        match kind {
            FsErrorKind::Busy => Self::Busy,
            FsErrorKind::Denied => Self::Denied,
            FsErrorKind::NotFound => Self::Gone,
            _ => Self::Error(message.to_string()),
        }
    }
}

/// Об'єкт плану: один корінь правила.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanItem {
    /// Індекс правила в наборі.
    pub rule: usize,
    /// Ідентифікатор правила.
    pub rule_id: String,
    /// Корінь.
    pub root: String,
    /// Вердикт.
    pub verdict: Verdict,
    /// Дія.
    pub action: Action,
    /// Файли.
    pub files: Vec<Candidate>,
    /// Підкаталоги, які прибрати, якщо спорожніють (від найглибших).
    pub dirs: Vec<String>,
}

impl PlanItem {
    /// Сума байтів.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

/// Пропущений під час планування / виконання об'єкт.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Skipped {
    /// Шлях.
    pub path: String,
    /// Причина.
    pub reason: SkipReason,
}

/// План очищення (результат сухого прогону).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CleanPlan {
    /// Об'єкти.
    pub items: Vec<PlanItem>,
    /// Що відкинуто ще на етапі плану.
    pub skipped: Vec<Skipped>,
    /// Служби, які треба тимчасово зупинити.
    pub services: Vec<String>,
}

impl CleanPlan {
    /// Байтів до видалення.
    pub fn delete_bytes(&self) -> u64 {
        self.items
            .iter()
            .filter(|i| i.action == Action::Delete)
            .map(PlanItem::bytes)
            .sum()
    }

    /// Байтів до карантину.
    pub fn quarantine_bytes(&self) -> u64 {
        self.items
            .iter()
            .filter(|i| i.action == Action::Quarantine)
            .map(PlanItem::bytes)
            .sum()
    }

    /// Файлів у плані.
    pub fn file_count(&self) -> usize {
        self.items.iter().map(|i| i.files.len()).sum()
    }

    /// Чи торкається план диска (`C`).
    pub fn touches_drive(&self, drive: char) -> bool {
        self.items
            .iter()
            .any(|i| path::drive_letter(&i.root).is_some_and(|d| d.eq_ignore_ascii_case(&drive)))
    }
}

/// Перевірка одного файлу безпосередньо перед операцією (і під час планування).
/// `canon` — кеш перевірених батьківських каталогів.
pub fn check_target(
    fs: &dyn FsApi,
    rules: &RuleSet,
    rule: usize,
    root: &str,
    target: &str,
    canon: &mut HashMap<String, bool>,
) -> Result<u64, SkipReason> {
    let p = path::normalize(target).map_err(|e| SkipReason::BadPath(e.to_string()))?;
    let root_is_target = path::eq_ci(&p, root);
    if !root_is_target && !path::is_strictly_within(&p, root) {
        return Err(SkipReason::OutsideRoot);
    }
    if !rules.guard().allows_delete(&p) {
        return Err(SkipReason::Blacklisted);
    }
    let cls = rules.classify(&p);
    let lr = rules.get(rule).ok_or(SkipReason::RuleMismatch)?;
    if cls.rule != Some(rule) || cls.verdict != lr.verdict() || !lr.deletable() {
        return Err(SkipReason::RuleMismatch);
    }
    let parent = path::parent(&p).ok_or(SkipReason::OutsideRoot)?;
    let parent_ok = match canon.get(&path::key(parent)) {
        Some(ok) => *ok,
        None => {
            let ok = fs
                .canonicalize(parent)
                .is_ok_and(|c| path::eq_ci(&c, parent));
            canon.insert(path::key(parent), ok);
            ok
        }
    };
    if !parent_ok {
        return Err(SkipReason::Redirected);
    }
    let meta = fs
        .metadata(&p)
        .map_err(|e| SkipReason::from_fs(e.kind, &e.message))?;
    if meta.reparse {
        return Err(SkipReason::Reparse);
    }
    if meta.kind != Kind::File {
        return Err(SkipReason::Changed);
    }
    Ok(meta.len)
}

/// Сухий прогін: точний список файлів для вибраних коренів. Файлову систему не змінює.
///
/// `selected` — шляхи об'єктів зі звіту скану; кожен має бути коренем правила з методом delete.
pub fn build_plan(
    fs: &dyn FsApi,
    rules: &RuleSet,
    selected: &[String],
    now_unix: i64,
    cancel: &CancelToken,
) -> Result<CleanPlan, CleanupError> {
    let mut plan = CleanPlan::default();
    let mut canon = HashMap::new();
    for sel in selected {
        let root =
            path::normalize(sel).map_err(|e| CleanupError::NotDeletable(format!("{sel}: {e}")))?;
        let cls = rules.classify(&root);
        let rule = cls.rule.ok_or_else(|| {
            CleanupError::NotDeletable(format!("{root}: не збігається з жодним правилом"))
        })?;
        let lr = rules
            .get(rule)
            .ok_or_else(|| CleanupError::NotDeletable(root.clone()))?;
        if !lr.deletable() || cls.verdict != lr.verdict() {
            return Err(CleanupError::NotDeletable(format!(
                "{root}: правило «{}» не дозволяє видалення",
                lr.rule.id
            )));
        }
        // Вибраний шлях має бути саме коренем правила (не довільна тека всередині чи поза ним).
        let Some((root, meta)) = lr
            .patterns
            .iter()
            .flat_map(|p| expand_roots(fs, p))
            .find(|(r, _)| path::eq_ci(r, &root))
        else {
            return Err(CleanupError::NotDeletable(format!(
                "{root}: не корінь правила або веде через посилання"
            )));
        };
        if plan.items.iter().any(|i| path::eq_ci(&i.root, &root)) {
            continue;
        }
        let walk = walk_root(fs, lr, &root, &meta, now_unix, cancel);
        let mut files = Vec::with_capacity(walk.eligible.len());
        for c in walk.eligible {
            match check_target(fs, rules, rule, &root, &c.path, &mut canon) {
                Ok(_) => files.push(c),
                Err(reason) => plan.skipped.push(Skipped {
                    path: c.path,
                    reason,
                }),
            }
        }
        for s in &lr.rule.stop_services {
            if !plan.services.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                plan.services.push(s.clone());
            }
        }
        plan.items.push(PlanItem {
            rule,
            rule_id: lr.rule.id.clone(),
            root,
            verdict: lr.verdict(),
            action: if lr.verdict() == Verdict::Consequences {
                Action::Quarantine
            } else {
                Action::Delete
            },
            files,
            dirs: walk.dirs,
        });
    }
    Ok(plan)
}

/// Прогрес виконання.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExecProgress {
    /// Оброблено файлів.
    pub done: u64,
    /// Усього файлів.
    pub total: u64,
    /// Звільнено байтів.
    pub freed_bytes: u64,
    /// Перенесено в карантин байтів.
    pub quarantined_bytes: u64,
}

/// Зміна стану служби під час очищення.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServiceNote {
    /// Служба.
    pub name: String,
    /// Була запущена перед очищенням.
    pub was_running: bool,
    /// Стан відновлено.
    pub restored: bool,
    /// Помилка (якщо була).
    pub error: Option<String>,
}

/// Результат виконання.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExecResult {
    /// Видалено файлів / байтів.
    pub deleted_files: u64,
    /// Видалено байтів.
    pub freed_bytes: u64,
    /// Перенесено в карантин файлів.
    pub quarantined_files: u64,
    /// Перенесено в карантин байтів.
    pub quarantined_bytes: u64,
    /// Пропущено.
    pub skipped: Vec<Skipped>,
    /// Рядки для журналу.
    pub lines: Vec<JournalLine>,
    /// Служби.
    pub services: Vec<ServiceNote>,
    /// Партія карантину.
    pub batch_id: Option<String>,
    /// Скасовано посередині.
    pub cancelled: bool,
}

/// Оточення виконання.
#[derive(Clone, Copy)]
pub struct ExecContext<'a> {
    /// Файлова система.
    pub fs: &'a dyn FsApi,
    /// Правила.
    pub rules: &'a RuleSet,
    /// Служби (потрібні, якщо правило зупиняє службу).
    pub services: Option<&'a dyn ServiceApi>,
    /// Поточний час (Unix-секунди).
    pub now_unix: i64,
    /// Скасування.
    pub cancel: &'a CancelToken,
}

/// Виконує план. Служби з плану зупиняються й після роботи повертаються в попередній стан,
/// навіть якщо щось пішло не так. Процеси не завершуються: зайняті файли пропускаються.
pub fn execute(
    ctx: &ExecContext<'_>,
    plan: &CleanPlan,
    quarantine: Option<&mut Quarantine<'_>>,
    progress: &dyn Fn(ExecProgress),
) -> Result<ExecResult, CleanupError> {
    let ExecContext {
        fs,
        rules,
        services,
        now_unix,
        cancel,
    } = *ctx;
    let mut res = ExecResult::default();
    let needs_quarantine = plan.items.iter().any(|i| i.action == Action::Quarantine);
    let mut quarantine = quarantine;
    if needs_quarantine && quarantine.is_none() {
        return Err(CleanupError::NotDeletable("карантин недоступний".into()));
    }
    // 1. Служби.
    let mut stopped: Vec<ServiceNote> = Vec::new();
    if !plan.services.is_empty() {
        let api = services
            .ok_or_else(|| CleanupError::NotDeletable("керування службами недоступне".into()))?;
        for name in &plan.services {
            let was_running = matches!(api.state(name), Ok(ServiceState::Running));
            let mut note = ServiceNote {
                name: name.clone(),
                was_running,
                restored: !was_running,
                error: None,
            };
            if was_running {
                if let Err(e) = api.stop(name) {
                    // Не вдалося зупинити — файли служби, найімовірніше, зайняті; продовжуємо,
                    // зайняте буде пропущено.
                    note.error = Some(e.to_string());
                }
            }
            stopped.push(note);
        }
    }
    // 2. Файли.
    let total = plan.file_count() as u64;
    let mut done = 0u64;
    let mut canon = HashMap::new();
    let mut batch = match (&mut quarantine, needs_quarantine) {
        (Some(q), true) => Some(q.begin_batch(now_unix)),
        _ => None,
    };
    'items: for item in &plan.items {
        for f in &item.files {
            if cancel.is_cancelled() {
                res.cancelled = true;
                break 'items;
            }
            done += 1;
            let outcome = match check_target(fs, rules, item.rule, &item.root, &f.path, &mut canon)
            {
                Err(reason) => Err(reason),
                Ok(len) => match item.action {
                    Action::Delete => fs
                        .remove_file(&f.path)
                        .map(|()| len)
                        .map_err(|e| SkipReason::from_fs(e.kind, &e.message)),
                    Action::Quarantine => match (&mut quarantine, &mut batch) {
                        (Some(q), Some(b)) => q
                            .put(b, &f.path, &item.rule_id, len)
                            .map(|()| len)
                            .map_err(|e| SkipReason::from_fs(e.kind, &e.message)),
                        _ => Err(SkipReason::Error("карантин недоступний".into())),
                    },
                },
            };
            let line_outcome = match outcome {
                Ok(len) => {
                    if item.action == Action::Delete {
                        res.deleted_files += 1;
                        res.freed_bytes += len;
                        Outcome::Deleted
                    } else {
                        res.quarantined_files += 1;
                        res.quarantined_bytes += len;
                        Outcome::Quarantined
                    }
                }
                Err(reason) => {
                    let o = Outcome::Skipped {
                        reason: reason.describe(),
                    };
                    res.skipped.push(Skipped {
                        path: f.path.clone(),
                        reason,
                    });
                    o
                }
            };
            res.lines.push(JournalLine {
                path: f.path.clone(),
                rule_id: Some(item.rule_id.clone()),
                bytes: f.bytes,
                outcome: line_outcome,
            });
            if done.is_multiple_of(64) || done == total {
                progress(ExecProgress {
                    done,
                    total,
                    freed_bytes: res.freed_bytes,
                    quarantined_bytes: res.quarantined_bytes,
                });
            }
        }
        // Спорожнілі підкаталоги (сам корінь лишається). Лише справжні каталоги в межах кореня.
        for d in &item.dirs {
            let inside = path::is_strictly_within(d, &item.root) && rules.guard().allows_delete(d);
            let plain = fs
                .metadata(d)
                .is_ok_and(|m| m.kind == Kind::Dir && !m.reparse);
            if inside && plain {
                let _ = fs.remove_empty_dir(d);
            }
        }
    }
    if let (Some(q), Some(b)) = (&mut quarantine, batch) {
        res.batch_id = q.finish_batch(b)?;
    }
    // 3. Служби — назад у попередній стан.
    if let Some(api) = services {
        for note in &mut stopped {
            if note.was_running {
                match api.start(&note.name) {
                    Ok(()) => note.restored = true,
                    Err(e) => note.error = Some(e.to_string()),
                }
            }
        }
    }
    res.services = stopped;
    progress(ExecProgress {
        done,
        total,
        freed_bytes: res.freed_bytes,
        quarantined_bytes: res.quarantined_bytes,
    });
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::MemFs;
    use crate::pattern::Env;
    use optimax_core::service::MemServices;

    const NOW: i64 = 1_800_000_000;
    const OLD: i64 = NOW - 30 * 86_400;
    const TEMP: &str = r"C:\Users\Test\AppData\Local\Temp";
    const DX: &str = r"C:\Users\Test\AppData\Local\NVIDIA\DXCache";

    fn rules() -> RuleSet {
        RuleSet::embedded(Env::for_tests()).0
    }

    fn fs() -> MemFs {
        let fs = MemFs::new();
        fs.add_file(&format!(r"{TEMP}\a.tmp"), 100, OLD)
            .add_file(&format!(r"{TEMP}\sub\b.tmp"), 50, OLD)
            .add_file(&format!(r"{TEMP}\new.tmp"), 7, NOW)
            .add_file(&format!(r"{DX}\shader.nvph"), 400, OLD)
            .add_file(r"C:\Windows\System32\kernel32.dll", 10, OLD)
            .add_file(r"C:\Users\Test\Documents\thesis.docx", 10, OLD)
            .add_file(
                r"C:\Windows\SoftwareDistribution\Download\pkg.cab",
                900,
                OLD,
            )
            .add_file(r"C:\Users\Test\AppData\Local\Unknown\big.bin", 10, OLD);
        fs
    }

    fn run(
        fs: &MemFs,
        rs: &RuleSet,
        plan: &CleanPlan,
        services: Option<&MemServices>,
    ) -> Result<ExecResult, CleanupError> {
        let cancel = CancelToken::new();
        let ctx = ExecContext {
            fs,
            rules: rs,
            services: services.map(|s| s as &dyn ServiceApi),
            now_unix: NOW,
            cancel: &cancel,
        };
        execute(&ctx, plan, None, &|_| {})
    }

    fn sel(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn dry_run_changes_nothing() {
        let fs = fs();
        let plan = build_plan(&fs, &rules(), &sel(&[TEMP, DX]), NOW, &CancelToken::new()).unwrap();
        assert_eq!(plan.delete_bytes(), 150);
        assert_eq!(plan.quarantine_bytes(), 400);
        assert_eq!(fs.mutation_count(), 0, "{:?}", fs.mutations());
    }

    #[test]
    fn execute_deletes_only_planned_files() {
        let fs = fs();
        let rs = rules();
        let plan = build_plan(&fs, &rs, &sel(&[TEMP]), NOW, &CancelToken::new()).unwrap();
        // Після сухого прогону з'явився новий старий файл — його в плані немає, тож не видаляється.
        fs.add_file(&format!(r"{TEMP}\late.tmp"), 5, OLD);
        let res = run(&fs, &rs, &plan, None).unwrap();
        assert_eq!(res.freed_bytes, 150);
        assert!(!fs.exists(&format!(r"{TEMP}\a.tmp")));
        assert!(
            !fs.exists(&format!(r"{TEMP}\sub")),
            "спорожнілий підкаталог прибрано"
        );
        assert!(fs.exists(TEMP), "корінь лишається");
        assert!(
            fs.exists(&format!(r"{TEMP}\new.tmp")),
            "свіжий файл лишається"
        );
        assert!(
            fs.exists(&format!(r"{TEMP}\late.tmp")),
            "файлу не було в плані"
        );
        assert!(fs.exists(r"C:\Windows\System32\kernel32.dll"));
        assert!(fs.exists(r"C:\Users\Test\Documents\thesis.docx"));
        assert!(fs.exists(r"C:\Users\Test\AppData\Local\Unknown\big.bin"));
    }

    #[test]
    fn unknown_and_user_files_are_never_planned() {
        let fs = fs();
        let rs = rules();
        for bad in [
            r"C:\Users\Test\AppData\Local\Unknown",
            r"C:\Users\Test\Documents",
            r"C:\Windows\System32",
            r"C:\Windows\WinSxS",
            r"C:\Users\Test\AppData\Local\Temp\sub",
            r"C:\Users\Test\AppData\Local\Temp\..\Unknown",
            r"C:\",
        ] {
            assert!(
                build_plan(&fs, &rs, &sel(&[bad]), NOW, &CancelToken::new()).is_err(),
                "{bad}"
            );
        }
        assert_eq!(fs.mutation_count(), 0);
    }

    #[test]
    fn junction_inside_root_is_not_followed() {
        let fs = fs();
        fs.add_link(&format!(r"{TEMP}\evil"), r"C:\Windows\System32", true);
        fs.add_link(&format!(r"{TEMP}\docs"), r"C:\Users\Test\Documents", true);
        let rs = rules();
        let plan = build_plan(&fs, &rs, &sel(&[TEMP]), NOW, &CancelToken::new()).unwrap();
        assert!(plan.items[0]
            .files
            .iter()
            .all(|f| !f.path.contains("evil") && !f.path.contains("docs")));
        run(&fs, &rs, &plan, None).unwrap();
        assert!(fs.exists(r"C:\Windows\System32\kernel32.dll"));
        assert!(fs.exists(r"C:\Users\Test\Documents\thesis.docx"));
        assert!(
            fs.exists(&format!(r"{TEMP}\evil")),
            "саме посилання теж не чіпаємо"
        );
    }

    #[test]
    fn root_that_is_a_junction_is_refused() {
        let fs = MemFs::new();
        fs.add_file(r"C:\Users\Test\Documents\thesis.docx", 10, OLD);
        fs.add_link(TEMP, r"C:\Users\Test\Documents", true);
        let r = build_plan(&fs, &rules(), &sel(&[TEMP]), NOW, &CancelToken::new());
        assert!(r.is_err());
        assert!(fs.exists(r"C:\Users\Test\Documents\thesis.docx"));
    }

    #[test]
    fn swapped_for_junction_after_dry_run_is_skipped() {
        let fs = fs();
        let rs = rules();
        let plan = build_plan(&fs, &rs, &sel(&[TEMP]), NOW, &CancelToken::new()).unwrap();
        // Між сухим прогоном і виконанням підкаталог підмінили на junction у System32.
        fs.remove_file(&format!(r"{TEMP}\sub\b.tmp")).unwrap();
        fs.remove_empty_dir(&format!(r"{TEMP}\sub")).unwrap();
        fs.add_link(&format!(r"{TEMP}\sub"), r"C:\Windows\System32", true);
        fs.add_file(r"C:\Windows\System32\b.tmp", 50, OLD);
        let res = run(&fs, &rs, &plan, None).unwrap();
        assert!(fs.exists(r"C:\Windows\System32\b.tmp"));
        assert!(res
            .skipped
            .iter()
            .any(|s| s.reason == SkipReason::Redirected));
    }

    #[test]
    fn short_name_path_is_redirected() {
        let fs = fs();
        let rs = rules();
        fs.add_alias(r"C:\Users\Test\AppData\Local\TEMP~1", TEMP);
        let mut canon = HashMap::new();
        let idx = rs.index_of("user_temp").unwrap();
        let r = check_target(
            &fs,
            &rs,
            idx,
            TEMP,
            r"C:\Users\Test\AppData\Local\TEMP~1\a.tmp",
            &mut canon,
        );
        assert!(r.is_err());
    }

    #[test]
    fn traversal_and_mixed_separators_rejected() {
        let fs = fs();
        let rs = rules();
        let idx = rs.index_of("user_temp").unwrap();
        let mut canon = HashMap::new();
        for p in [
            r"C:\Users\Test\AppData\Local\Temp\..\..\..\Documents\thesis.docx",
            r"C:/Users/Test/AppData/Local/Temp/../Unknown/big.bin",
            r"C:\Users\Test\AppData\Local\Temp\a.tmp:hidden",
        ] {
            assert!(
                check_target(&fs, &rs, idx, TEMP, p, &mut canon).is_err(),
                "{p}"
            );
        }
        // Змішані роздільники без «..» — той самий файл, дозволено.
        assert!(check_target(
            &fs,
            &rs,
            idx,
            TEMP,
            r"C:/Users/Test/AppData/Local/Temp/a.tmp",
            &mut canon
        )
        .is_ok());
    }

    #[test]
    fn locked_file_is_skipped_without_failing() {
        let fs = fs();
        let rs = rules();
        fs.lock_file(&format!(r"{TEMP}\a.tmp"));
        let plan = build_plan(&fs, &rs, &sel(&[TEMP]), NOW, &CancelToken::new()).unwrap();
        let res = run(&fs, &rs, &plan, None).unwrap();
        assert!(fs.exists(&format!(r"{TEMP}\a.tmp")));
        assert_eq!(res.freed_bytes, 50);
        assert!(res.skipped.iter().any(|s| s.reason == SkipReason::Busy));
    }

    #[test]
    fn services_are_restored() {
        let fs = fs();
        let rs = rules();
        let svc = MemServices::new(&[("wuauserv", ServiceState::Running)]);
        let wu = r"C:\Windows\SoftwareDistribution\Download";
        let plan = build_plan(&fs, &rs, &sel(&[wu]), NOW, &CancelToken::new()).unwrap();
        assert_eq!(plan.services, vec!["wuauserv".to_string()]);
        let res = run(&fs, &rs, &plan, Some(&svc)).unwrap();
        assert_eq!(res.freed_bytes, 900);
        assert_eq!(svc.state("wuauserv").unwrap(), ServiceState::Running);
        assert!(res.services[0].restored);
        // Служба була зупинена — після очищення лишається зупиненою.
        let fs2 = self::fs();
        let svc = MemServices::new(&[("wuauserv", ServiceState::Stopped)]);
        let plan = build_plan(&fs2, &rs, &sel(&[wu]), NOW, &CancelToken::new()).unwrap();
        run(&fs2, &rs, &plan, Some(&svc)).unwrap();
        assert_eq!(svc.state("wuauserv").unwrap(), ServiceState::Stopped);
    }

    #[test]
    fn consequences_need_quarantine() {
        let fs = fs();
        let rs = rules();
        let plan = build_plan(&fs, &rs, &sel(&[DX]), NOW, &CancelToken::new()).unwrap();
        assert_eq!(plan.items[0].action, Action::Quarantine);
        assert!(run(&fs, &rs, &plan, None).is_err());
        assert!(fs.exists(&format!(r"{DX}\shader.nvph")));
    }
}
