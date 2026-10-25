//! Скан: розміри за правилами бази й (повний режим) паралельний обхід диска, щоб показати великі
//! теки, яких база не знає. Читаються лише метадані; reparse points не обходяться, тож скан не
//! виходить за межі диска й не заходить у junction.

use crate::fs::{FsApi, Kind, Meta};
use crate::guard::Zone;
use crate::path;
use crate::pattern::Pattern;
use crate::rules::{Category, LoadedRule, Method, RuleSet, Verdict};
use crate::CancelToken;
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Великі невідомі теки — від цього розміру.
pub const UNKNOWN_MIN_BYTES: u64 = 512 * 1024 * 1024;
/// Скільки невідомих тек показувати (і окремо — скільки тек із зон «не чіпати»).
pub const UNKNOWN_LIMIT: usize = 30;
/// Скільки найбільших дрібних підтек показувати в рядку повного скану.
pub const SUBDIRS_SHOWN: usize = 8;
/// У знімок розмірів потрапляють теки від цього розміру…
pub const SNAPSHOT_MIN_BYTES: u64 = 100 * 1024 * 1024;
/// …і не глибше цього рівня.
pub const SNAPSHOT_MAX_DEPTH: usize = 5;

/// Опис правила для показу (без внутрішніх полів).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RuleInfo {
    /// Ідентифікатор.
    pub id: String,
    /// Що це.
    pub description: String,
    /// Що буде після видалення.
    pub consequences: String,
    /// Чи відновиться саме.
    pub regenerates: bool,
    /// Джерело.
    pub source: String,
    /// На чому перевірено.
    pub verified_on: String,
    /// Як очистити штатно.
    pub manual_hint: Option<String>,
    /// Правило понижене до ⚪ через помилку валідації.
    pub demoted: bool,
    /// Умова правила: видаляються лише файли, яких не чіпали стільки днів.
    pub min_age_days: Option<u32>,
}

impl RuleInfo {
    fn of(r: &LoadedRule) -> Self {
        Self {
            id: r.rule.id.clone(),
            description: r.rule.description.clone(),
            consequences: r.rule.consequences.clone(),
            regenerates: r.rule.regenerates,
            source: r.rule.source.clone().unwrap_or_default(),
            verified_on: r.rule.verified_on.clone().unwrap_or_default(),
            manual_hint: r.rule.manual_hint.clone(),
            demoted: r.demoted,
            min_age_days: r.rule.min_age_days,
        }
    }
}

/// Підтека рядка повного скану (менша за поріг окремого рядка).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SubDir {
    /// Шлях.
    pub path: String,
    /// Байтів.
    pub bytes: u64,
}

/// Об'єкт у звіті скану.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Item {
    /// Шлях (корінь правила або тека з повного скану).
    pub path: String,
    /// Категорія.
    pub category: Category,
    /// Вердикт.
    pub verdict: Verdict,
    /// Спосіб очищення.
    pub method: Method,
    /// Усього байтів.
    pub bytes: u64,
    /// Усього файлів.
    pub files: u64,
    /// Байтів, що підпадають під умови правила (вік, шаблон імені, «залишити останню»).
    pub eligible_bytes: u64,
    /// Файлів, що підпадають під умови правила.
    pub eligible_files: u64,
    /// Можна вибрати для очищення (🟢 / 🟡 з методом delete).
    pub selectable: bool,
    /// Правило дозволяє видалення, але зараз під його умови не підпадає жоден файл
    /// (напр., усі файли використовувались нещодавно), тож вибрати нічого.
    pub nothing_eligible: bool,
    /// Правило (немає — невідоме).
    pub rule: Option<RuleInfo>,
    /// Зона чорного списку.
    pub zone: Option<Zone>,
    /// Розмір — лише файли самої теки та дрібних підтек (великі підтеки показано окремо).
    pub remainder: bool,
    /// Повний скан: скільки дрібних підтек входить у розмір.
    pub small_dirs: u64,
    /// Повний скан: найбільші з цих дрібних підтек (до [`SUBDIRS_SHOWN`]).
    pub top_subdirs: Vec<SubDir>,
    /// Каталогів, які не вдалося прочитати (немає доступу).
    pub unreadable_dirs: u64,
}

/// Звіт скану.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScanReport {
    /// Корінь диска (`C:\`).
    pub drive: String,
    /// Повний скан диска (інакше — лише за правилами).
    pub full: bool,
    /// Коли почато (Unix-секунди).
    pub started_unix: i64,
    /// Тривалість, мс.
    pub duration_ms: u64,
    /// Об'єкти за правилами.
    pub items: Vec<Item>,
    /// Великі теки, яких база не знає (лише повний скан, лише ⚪).
    pub unknown: Vec<Item>,
    /// Великі теки в зонах чорного списку — системні, програми, особисті файли (лише повний
    /// скан). Що це — відомо, але правила немає й видалення неможливе.
    pub protected_zones: Vec<Item>,
    /// Усього байтів, побачених повним сканом.
    pub scanned_bytes: u64,
    /// Каталогів, які не вдалося прочитати.
    pub unreadable_dirs: u64,
    /// Скасовано (звіт неповний).
    pub cancelled: bool,
    /// Розміри для знімка «Що виросло» (у UI не передаються).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub sizes: Vec<(String, u64)>,
}

/// Фаза скану.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ScanPhase {
    /// Об'єкти за правилами.
    Rules,
    /// Обхід усього диска.
    Drive,
}

/// Прогрес скану.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScanProgress {
    /// Фаза.
    pub phase: ScanPhase,
    /// Каталогів оброблено.
    pub dirs: u64,
    /// Файлів побачено.
    pub files: u64,
    /// Байтів побачено.
    pub bytes: u64,
    /// Поточний каталог.
    pub current: String,
}

/// Налаштування скану.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Корінь диска (`C:\`).
    pub drive: String,
    /// Повний обхід диска.
    pub full: bool,
    /// Поточний час (Unix-секунди) — для умови «старше N днів».
    pub now_unix: i64,
    /// Потоків для повного обходу.
    pub threads: usize,
}

/// Розгортає шаблон правила в наявні корені: `*` — лише справжні каталоги (не посилання).
/// Корінь, канонічний шлях якого відрізняється (junction чи коротке ім'я по дорозі), пропускається.
pub fn expand_roots(fs: &dyn FsApi, pat: &Pattern) -> Vec<(String, Meta)> {
    let mut cur = vec![pat.drive().to_string()];
    for seg in pat.segs() {
        let mut next = Vec::new();
        for base in &cur {
            match seg {
                Some(name) => next.push(path::join(base, name)),
                None => {
                    if let Ok(entries) = fs.read_dir(base) {
                        next.extend(
                            entries
                                .into_iter()
                                .filter(|e| e.meta.kind == Kind::Dir && !e.meta.reparse)
                                .map(|e| path::join(base, &e.name)),
                        );
                    }
                }
            }
        }
        cur = next;
    }
    cur.into_iter()
        .filter_map(|root| {
            let meta = fs.metadata(&root).ok()?;
            if meta.reparse {
                return None;
            }
            let canonical = fs.canonicalize(&root).ok()?;
            path::eq_ci(&canonical, &root).then_some((root, meta))
        })
        .collect()
}

/// Файл, що підпадає під правило.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Шлях.
    pub path: String,
    /// Розмір.
    pub bytes: u64,
}

/// Результат обходу кореня правила.
#[derive(Debug, Default, Clone)]
pub struct RootWalk {
    /// Усього байтів / файлів.
    pub bytes: u64,
    /// Усього файлів.
    pub files: u64,
    /// Файли, що підпадають під умови правила.
    pub eligible: Vec<Candidate>,
    /// Підкаталоги, у яких були такі файли (від найглибших) — кандидати на видалення, якщо спорожніють.
    pub dirs: Vec<String>,
    /// Каталогів, які не вдалося прочитати.
    pub unreadable_dirs: u64,
    /// Пропущені reparse points.
    pub reparse_skipped: u64,
}

/// Прогрес обходу кореня: файлів і байтів побачено досі.
pub type WalkTick<'a> = dyn Fn(u64, u64) + Sync + 'a;

/// Як часто повідомляти про прогрес під час обходу.
const TICK_EVERY: Duration = Duration::from_millis(200);

/// Обходить корінь правила (каталог або файл), не заходячи в reparse points, і відбирає файли
/// за умовами правила. Нічого не змінює. Один потік, без прогресу.
pub fn walk_root(
    fs: &dyn FsApi,
    rule: &LoadedRule,
    root: &str,
    root_meta: &Meta,
    now_unix: i64,
    cancel: &CancelToken,
) -> RootWalk {
    walk_root_with(fs, rule, root, root_meta, now_unix, 1, cancel, &|_, _| {})
}

/// Те саме, що [`walk_root`], але підкаталоги обходять `threads` потоків, а `tick` отримує
/// прогрес не частіше ніж раз на 200 мс. Результат не залежить від кількості потоків.
#[allow(clippy::too_many_arguments)]
pub fn walk_root_with(
    fs: &dyn FsApi,
    rule: &LoadedRule,
    root: &str,
    root_meta: &Meta,
    now_unix: i64,
    threads: usize,
    cancel: &CancelToken,
    tick: &WalkTick<'_>,
) -> RootWalk {
    let mut out = RootWalk::default();
    let mut filter = RootFilter {
        rule,
        root,
        now_unix,
        // Для 🔴 / ⚪ і штатних способів список файлів не потрібен — лише розмір.
        collect: rule.deletable(),
        kept: Vec::new(),
    };
    if root_meta.kind == Kind::File {
        out.bytes = root_meta.len;
        out.files = 1;
        if filter.takes(path::file_name(root), root_meta, false) {
            out.eligible.push(Candidate {
                path: root.to_string(),
                bytes: root_meta.len,
            });
        }
        return out;
    }
    let top = match fs.read_dir(root) {
        Ok(t) => t,
        Err(_) => {
            out.unreadable_dirs += 1;
            return out;
        }
    };
    // «Залишити N найновіших» — серед прямих нащадків кореня.
    if let Some(n) = rule.rule.keep_latest {
        let mut by_age: Vec<_> = top.iter().filter(|e| !e.meta.reparse).collect();
        by_age.sort_by_key(|e| std::cmp::Reverse(e.meta.newest_unix));
        filter.kept = by_age
            .iter()
            .take(usize::try_from(n).unwrap_or(usize::MAX))
            .map(|e| path::key(&path::join(root, &e.name)))
            .collect();
    }
    let shared = WalkShared {
        stack: Mutex::new(vec![Job {
            dir: root.to_string(),
            entries: Some(top),
            keep: false,
        }]),
        wake: Condvar::new(),
        pending: AtomicUsize::new(1),
        files: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
        last_tick: Mutex::new(Instant::now()),
    };
    let threads = threads.clamp(1, 16);
    let parts: Vec<WalkPart> = if threads == 1 {
        vec![walk_worker(fs, &filter, &shared, cancel, tick)]
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|_| s.spawn(|| walk_worker(fs, &filter, &shared, cancel, tick)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    // Потік, що впав, втрачає свою частину — позначаємо це як неприступний каталог.
                    h.join().unwrap_or_else(|_| WalkPart {
                        unreadable_dirs: 1,
                        ..WalkPart::default()
                    })
                })
                .collect()
        })
    };
    let mut dirs_with_candidates: Vec<String> = Vec::new();
    for p in parts {
        out.bytes += p.bytes;
        out.files += p.files;
        out.unreadable_dirs += p.unreadable_dirs;
        out.reparse_skipped += p.reparse_skipped;
        out.eligible.extend(p.eligible);
        dirs_with_candidates.extend(p.dirs_with_candidates);
    }
    // Порядок потоків випадковий — сортуємо, щоб сухий прогін і план були відтворюваними.
    out.eligible.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    // Від найглибших до верхніх, плюс їхні батьки в межах кореня.
    let mut seen: HashSet<String> = HashSet::new();
    let mut all: Vec<String> = Vec::new();
    for d in dirs_with_candidates {
        let mut cur = Some(d.as_str());
        while let Some(c) = cur {
            if path::eq_ci(c, root) || !path::is_within(c, root) {
                break;
            }
            // Якщо каталог уже є, то є й усі його предки.
            if !seen.insert(path::key(c)) {
                break;
            }
            all.push(c.to_string());
            cur = path::parent(c);
        }
    }
    all.sort_by(|a, b| path::depth(b).cmp(&path::depth(a)).then_with(|| a.cmp(b)));
    out.dirs = all;
    out
}

/// Умови правила для одного кореня.
struct RootFilter<'a> {
    rule: &'a LoadedRule,
    root: &'a str,
    now_unix: i64,
    collect: bool,
    /// Ключі прямих нащадків кореня під «залишити N найновіших».
    kept: Vec<String>,
}

impl RootFilter<'_> {
    /// Файл підпадає під правило.
    fn takes(&self, name: &str, meta: &Meta, keep: bool) -> bool {
        let r = &self.rule.rule;
        let old_enough = match r.min_age_days {
            Some(days) => self.now_unix - meta.newest_unix >= i64::from(days) * 86_400,
            None => true,
        };
        let name_ok =
            r.file_patterns.is_empty() || r.file_patterns.iter().any(|p| path::glob_name(p, name));
        self.collect && !keep && old_enough && name_ok
    }

    fn is_kept(&self, p: &str) -> bool {
        !self.kept.is_empty() && self.kept.contains(&path::key(p))
    }
}

/// Каталог до обходу.
struct Job {
    dir: String,
    /// Вміст уже прочитано (корінь).
    entries: Option<Vec<crate::fs::Entry>>,
    /// Під «залишити N найновіших».
    keep: bool,
}

/// Частина результату одного потоку.
#[derive(Default)]
struct WalkPart {
    bytes: u64,
    files: u64,
    eligible: Vec<Candidate>,
    dirs_with_candidates: Vec<String>,
    unreadable_dirs: u64,
    reparse_skipped: u64,
}

struct WalkShared {
    /// Стек, а не черга: обхід у глибину тримає менше шляхів у пам'яті.
    stack: Mutex<Vec<Job>>,
    wake: Condvar,
    pending: AtomicUsize,
    files: AtomicU64,
    bytes: AtomicU64,
    last_tick: Mutex<Instant>,
}

fn walk_worker(
    fs: &dyn FsApi,
    f: &RootFilter<'_>,
    sh: &WalkShared,
    cancel: &CancelToken,
    tick: &WalkTick<'_>,
) -> WalkPart {
    let mut part = WalkPart::default();
    let mut subdirs: Vec<Job> = Vec::new();
    loop {
        let job = {
            let mut st = sh.stack.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if cancel.is_cancelled() {
                    return part;
                }
                if let Some(j) = st.pop() {
                    break j;
                }
                if sh.pending.load(Ordering::Acquire) == 0 {
                    sh.wake.notify_all();
                    return part;
                }
                st = sh
                    .wake
                    .wait_timeout(st, Duration::from_millis(20))
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
        };
        let (files0, bytes0) = (part.files, part.bytes);
        visit(fs, f, job, &mut part, &mut subdirs);
        if !subdirs.is_empty() {
            let n = subdirs.len();
            let mut st = sh.stack.lock().unwrap_or_else(PoisonError::into_inner);
            sh.pending.fetch_add(n, Ordering::AcqRel);
            st.append(&mut subdirs);
            drop(st);
            sh.wake.notify_all();
        }
        if sh.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            // Останній каталог — будимо тих, хто чекає, щоб вони завершились.
            sh.wake.notify_all();
        }
        let (df, db) = (part.files - files0, part.bytes - bytes0);
        let files = sh.files.fetch_add(df, Ordering::Relaxed) + df;
        let bytes = sh.bytes.fetch_add(db, Ordering::Relaxed) + db;
        let mut last = sh.last_tick.lock().unwrap_or_else(PoisonError::into_inner);
        if last.elapsed() >= TICK_EVERY {
            *last = Instant::now();
            drop(last);
            tick(files, bytes);
        }
    }
}

/// Читає один каталог: рахує файли, відбирає кандидатів, підкаталоги — у `subdirs`.
fn visit(
    fs: &dyn FsApi,
    f: &RootFilter<'_>,
    job: Job,
    part: &mut WalkPart,
    subdirs: &mut Vec<Job>,
) {
    let entries = match job.entries {
        Some(e) => e,
        None => match fs.read_dir(&job.dir) {
            Ok(e) => e,
            Err(_) => {
                part.unreadable_dirs += 1;
                return;
            }
        },
    };
    let mut had_candidate = false;
    for e in entries {
        if e.meta.reparse {
            part.reparse_skipped += 1;
            continue;
        }
        let p = path::join(&job.dir, &e.name);
        let keep = job.keep || f.is_kept(&p);
        match e.meta.kind {
            Kind::Dir => subdirs.push(Job {
                dir: p,
                entries: None,
                keep,
            }),
            Kind::File => {
                part.bytes += e.meta.len;
                part.files += 1;
                if f.takes(&e.name, &e.meta, keep) {
                    part.eligible.push(Candidate {
                        path: p,
                        bytes: e.meta.len,
                    });
                    had_candidate = true;
                }
            }
        }
    }
    if had_candidate && !path::eq_ci(&job.dir, f.root) {
        part.dirs_with_candidates.push(job.dir);
    }
}

/// Скан диска за правилами (і, за потреби, повний обхід).
///
/// Спершу обходяться корені, які можна чистити (вони швидкі), потім 🔴 (WinSxS, Installer —
/// великі, лише для інформації). Після кожної з цих фаз `partial` отримує проміжний звіт, щоб
/// екран показав знайдене, не чекаючи кінця скану; остаточний звіт — результат функції.
pub fn scan(
    fs: &dyn FsApi,
    rules: &RuleSet,
    opts: &ScanOptions,
    cancel: &CancelToken,
    progress: &(dyn Fn(ScanProgress) + Sync),
    partial: &dyn Fn(&ScanReport),
) -> ScanReport {
    let started = Instant::now();
    let mut report = ScanReport {
        drive: opts.drive.clone(),
        full: opts.full,
        started_unix: opts.now_unix,
        duration_ms: 0,
        items: Vec::new(),
        unknown: Vec::new(),
        protected_zones: Vec::new(),
        scanned_bytes: 0,
        unreadable_dirs: 0,
        cancelled: false,
        sizes: Vec::new(),
    };
    let elapsed_ms = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (mut dirs, mut files, mut bytes) = (0u64, 0u64, 0u64);
    for protected in [false, true] {
        let pass = rules
            .rules()
            .iter()
            .filter(|lr| (lr.verdict() == Verdict::Protected) == protected);
        for lr in pass {
            for pat in lr
                .patterns
                .iter()
                .filter(|p| path::eq_ci(p.drive(), &opts.drive))
            {
                for (root, meta) in expand_roots(fs, pat) {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let at = |f: u64, b: u64| ScanProgress {
                        phase: ScanPhase::Rules,
                        dirs,
                        files: files + f,
                        bytes: bytes + b,
                        current: root.clone(),
                    };
                    // Показуємо корінь, який обходимо зараз, а не попередній.
                    progress(at(0, 0));
                    // Корінь може збігтися зі ще суворішим правилом — тоді показуємо суворіший вердикт.
                    let cls = rules.classify(&root);
                    let walk = walk_root_with(
                        fs,
                        lr,
                        &root,
                        &meta,
                        opts.now_unix,
                        opts.threads,
                        cancel,
                        &|f, b| progress(at(f, b)),
                    );
                    dirs += 1;
                    files += walk.files;
                    bytes += walk.bytes;
                    if walk.files == 0 && walk.unreadable_dirs == 0 {
                        continue;
                    }
                    let verdict = cls.verdict.max(lr.verdict());
                    let eligible_bytes = walk.eligible.iter().map(|c| c.bytes).sum();
                    let may_delete = lr.deletable() && verdict == lr.verdict();
                    report.unreadable_dirs += walk.unreadable_dirs;
                    report.items.push(Item {
                        path: root,
                        category: lr.rule.category,
                        verdict,
                        method: lr.method(),
                        bytes: walk.bytes,
                        files: walk.files,
                        eligible_bytes,
                        eligible_files: walk.eligible.len() as u64,
                        selectable: may_delete && eligible_bytes > 0,
                        nothing_eligible: may_delete && eligible_bytes == 0 && walk.bytes > 0,
                        rule: Some(RuleInfo::of(lr)),
                        zone: cls.zone,
                        unreadable_dirs: walk.unreadable_dirs,
                        remainder: false,
                        small_dirs: 0,
                        top_subdirs: Vec::new(),
                    });
                }
            }
        }
        report.items.sort_by_key(|i| std::cmp::Reverse(i.bytes));
        // Після 🔴 проміжний звіт потрібен лише перед довгим обходом диска.
        if !cancel.is_cancelled() && (!protected || opts.full) {
            report.duration_ms = elapsed_ms();
            partial(&report);
        }
    }
    if opts.full && !cancel.is_cancelled() {
        let tree = walk_drive(fs, rules, opts, cancel, progress);
        report.scanned_bytes = tree.total(0);
        report.unreadable_dirs += tree.unreadable;
        (report.unknown, report.protected_zones) = tree.unknown(rules);
        report.sizes = tree.snapshot_sizes();
    }
    report
        .sizes
        .extend(report.items.iter().map(|i| (i.path.clone(), i.bytes)));
    report.cancelled = cancel.is_cancelled();
    report.duration_ms = elapsed_ms();
    report
}

// ---------------------------------------------------------------------------------------------
// Повний обхід диска
// ---------------------------------------------------------------------------------------------

struct Node {
    parent: u32,
    path: Box<str>,
    own: u64,
}

struct Tree {
    nodes: Vec<Node>,
    totals: Vec<u64>,
    children: Vec<Vec<u32>>,
    unreadable: u64,
}

impl Tree {
    fn total(&self, i: u32) -> u64 {
        self.totals.get(i as usize).copied().unwrap_or(0)
    }

    fn path(&self, i: u32) -> &str {
        self.nodes.get(i as usize).map(|n| &*n.path).unwrap_or("")
    }

    /// Великі теки без правила: спускаємось у великих нащадків, показуємо «місця концентрації».
    /// Повертає (невідоме ⚪, теки із зон «не чіпати»), кожне — до [`UNKNOWN_LIMIT`].
    fn unknown(&self, rules: &RuleSet) -> (Vec<Item>, Vec<Item>) {
        let mut found: Vec<(u32, u64, bool)> = Vec::new();
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let big: Vec<u32> = self.children[i as usize]
                .iter()
                .copied()
                .filter(|c| self.total(*c) >= UNKNOWN_MIN_BYTES)
                .collect();
            let rest = self.total(i) - big.iter().map(|c| self.total(*c)).sum::<u64>();
            let depth = path::depth(self.path(i));
            if big.is_empty() {
                if depth >= 1 {
                    found.push((i, self.total(i), false));
                }
            } else {
                if depth >= 2 && rest >= UNKNOWN_MIN_BYTES {
                    found.push((i, rest, true));
                }
                stack.extend(big);
            }
        }
        found.sort_by_key(|(_, b, _)| std::cmp::Reverse(*b));
        let (mut unknown, mut zones) = (Vec::new(), Vec::new());
        for (i, bytes, remainder) in found {
            if unknown.len() >= UNKNOWN_LIMIT && zones.len() >= UNKNOWN_LIMIT {
                break;
            }
            let p = self.path(i).to_string();
            let cls = rules.classify(&p);
            let list = if cls.zone.is_some() {
                &mut zones
            } else {
                &mut unknown
            };
            if list.len() >= UNKNOWN_LIMIT {
                continue;
            }
            // Дрібні підтеки, з яких складається розмір рядка (великі показано окремими рядками).
            let mut small: Vec<u32> = self.children[i as usize]
                .iter()
                .copied()
                .filter(|c| self.total(*c) < UNKNOWN_MIN_BYTES)
                .collect();
            small.sort_by_key(|c| std::cmp::Reverse(self.total(*c)));
            list.push(Item {
                path: p,
                category: Category::Unknown,
                verdict: if cls.zone.is_some() {
                    Verdict::Protected
                } else {
                    Verdict::Unknown
                },
                method: Method::ManualOnly,
                bytes,
                files: 0,
                eligible_bytes: 0,
                eligible_files: 0,
                selectable: false,
                nothing_eligible: false,
                rule: None,
                zone: cls.zone,
                unreadable_dirs: 0,
                remainder,
                small_dirs: small.len() as u64,
                top_subdirs: small
                    .iter()
                    .take(SUBDIRS_SHOWN)
                    .filter(|c| self.total(**c) > 0)
                    .map(|c| SubDir {
                        path: self.path(*c).to_string(),
                        bytes: self.total(*c),
                    })
                    .collect(),
            });
        }
        (unknown, zones)
    }

    fn snapshot_sizes(&self) -> Vec<(String, u64)> {
        (0..self.nodes.len())
            .filter_map(|i| {
                let i = u32::try_from(i).ok()?;
                let p = self.path(i);
                (path::depth(p) <= SNAPSHOT_MAX_DEPTH && self.total(i) >= SNAPSHOT_MIN_BYTES)
                    .then(|| (p.to_string(), self.total(i)))
            })
            .collect()
    }
}

struct Shared {
    queue: Mutex<VecDeque<(u32, String)>>,
    wake: Condvar,
    pending: AtomicUsize,
    nodes: Mutex<Vec<Node>>,
    dirs: AtomicU64,
    files: AtomicU64,
    bytes: AtomicU64,
    unreadable: AtomicU64,
    last_progress: Mutex<Instant>,
}

fn walk_drive(
    fs: &dyn FsApi,
    rules: &RuleSet,
    opts: &ScanOptions,
    cancel: &CancelToken,
    progress: &(dyn Fn(ScanProgress) + Sync),
) -> Tree {
    let root = opts.drive.clone();
    let shared = Shared {
        queue: Mutex::new(VecDeque::from([(0, root.clone())])),
        wake: Condvar::new(),
        pending: AtomicUsize::new(1),
        nodes: Mutex::new(vec![Node {
            parent: 0,
            path: root.into(),
            own: 0,
        }]),
        dirs: AtomicU64::new(0),
        files: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
        unreadable: AtomicU64::new(0),
        last_progress: Mutex::new(Instant::now()),
    };
    let threads = opts.threads.clamp(1, 16);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| drive_worker(fs, rules, &shared, cancel, progress));
        }
    });
    let nodes = shared
        .nodes
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let mut totals: Vec<u64> = nodes.iter().map(|n| n.own).collect();
    let mut children: Vec<Vec<u32>> = vec![Vec::new(); nodes.len()];
    // Нащадок завжди має більший індекс за батька — підсумовуємо з кінця.
    for i in (1..nodes.len()).rev() {
        let p = nodes[i].parent as usize;
        totals[p] += totals[i];
        if let Ok(idx) = u32::try_from(i) {
            children[p].push(idx);
        }
    }
    Tree {
        nodes,
        totals,
        children,
        unreadable: shared.unreadable.load(Ordering::Relaxed),
    }
}

fn drive_worker(
    fs: &dyn FsApi,
    rules: &RuleSet,
    sh: &Shared,
    cancel: &CancelToken,
    progress: &(dyn Fn(ScanProgress) + Sync),
) {
    let max_depth = rules.max_pattern_depth();
    loop {
        let job = {
            let mut q = sh.queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if cancel.is_cancelled() {
                    return;
                }
                if let Some(j) = q.pop_front() {
                    break j;
                }
                if sh.pending.load(Ordering::Acquire) == 0 {
                    sh.wake.notify_all();
                    return;
                }
                q = sh
                    .wake
                    .wait_timeout(q, Duration::from_millis(20))
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
        };
        let (idx, dir) = job;
        let mut own = 0u64;
        let mut subdirs: Vec<String> = Vec::new();
        match fs.read_dir(&dir) {
            Ok(entries) => {
                let mut nfiles = 0u64;
                // Предки цього каталогу не збіглися з правилами, тож дочірній об'єкт може належати
                // правилу, лише якщо він сам є коренем — а корінь не глибший за найдовший шаблон.
                let may_be_root = path::depth(&dir) < max_depth;
                let free = |p: &str| !may_be_root || rules.classify(p).rule.is_none();
                for e in entries {
                    if e.meta.reparse {
                        continue;
                    }
                    let p = path::join(&dir, &e.name);
                    match e.meta.kind {
                        // Каталоги, що належать правилам, рахує скан правил — тут їх пропускаємо.
                        Kind::Dir if free(&p) => subdirs.push(p),
                        Kind::Dir => {}
                        Kind::File => {
                            if free(&p) {
                                own += e.meta.len;
                                nfiles += 1;
                            }
                        }
                    }
                }
                sh.files.fetch_add(nfiles, Ordering::Relaxed);
                sh.bytes.fetch_add(own, Ordering::Relaxed);
            }
            Err(_) => {
                sh.unreadable.fetch_add(1, Ordering::Relaxed);
            }
        }
        {
            let mut nodes = sh.nodes.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(n) = nodes.get_mut(idx as usize) {
                n.own = own;
            }
            let mut q = sh.queue.lock().unwrap_or_else(PoisonError::into_inner);
            for p in subdirs {
                let Ok(child) = u32::try_from(nodes.len()) else {
                    break;
                };
                nodes.push(Node {
                    parent: idx,
                    path: p.clone().into(),
                    own: 0,
                });
                sh.pending.fetch_add(1, Ordering::AcqRel);
                q.push_back((child, p));
            }
        }
        sh.pending.fetch_sub(1, Ordering::AcqRel);
        sh.wake.notify_all();
        let n = sh.dirs.fetch_add(1, Ordering::Relaxed) + 1;
        let mut last = sh
            .last_progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.elapsed() >= Duration::from_millis(200) {
            *last = Instant::now();
            drop(last);
            progress(ScanProgress {
                phase: ScanPhase::Drive,
                dirs: n,
                files: sh.files.load(Ordering::Relaxed),
                bytes: sh.bytes.load(Ordering::Relaxed),
                current: dir,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::MemFs;
    use crate::pattern::Env;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;
    const GIB: u64 = 1024 * 1024 * 1024;

    fn rules() -> RuleSet {
        let (rs, issues) = RuleSet::embedded(Env::for_tests());
        assert!(issues.is_empty(), "{issues:?}");
        rs
    }

    fn opts(full: bool) -> ScanOptions {
        ScanOptions {
            drive: r"C:\".into(),
            full,
            now_unix: NOW,
            threads: 4,
        }
    }

    fn sample() -> MemFs {
        let fs = MemFs::new();
        let temp = r"C:\Users\Test\AppData\Local\Temp";
        fs.add_file(&format!(r"{temp}\old.tmp"), 1000, NOW - 30 * DAY)
            .add_file(&format!(r"{temp}\sub\old2.tmp"), 500, NOW - 10 * DAY)
            .add_file(&format!(r"{temp}\fresh.tmp"), 300, NOW - DAY)
            .add_file(
                r"C:\Users\Test\AppData\Local\Google\Chrome\User Data\Default\Cache\f_1",
                200,
                NOW,
            )
            .add_file(
                r"C:\Users\Test\AppData\Local\Google\Chrome\User Data\Profile 1\Cache\f_1",
                100,
                NOW,
            )
            .add_file(
                r"C:\Users\Test\AppData\Local\Google\Chrome\User Data\Default\History",
                999,
                NOW,
            )
            .add_file(r"C:\Games\Big\data.pak", 2 * GIB, NOW)
            .add_file(r"C:\Windows\WinSxS\x\comp.dll", 300, NOW)
            .add_file(r"C:\Windows\System32\kernel32.dll", 50, NOW);
        // Junction усередині Temp, що веде в System32: скан не повинен у нього заходити.
        fs.add_link(&format!(r"{temp}\evil"), r"C:\Windows\System32", true);
        fs
    }

    #[test]
    fn rule_scan_respects_age_and_wildcards() {
        let fs = sample();
        let report = scan(
            &fs,
            &rules(),
            &opts(false),
            &CancelToken::new(),
            &|_| {},
            &|_| {},
        );
        let temp = report
            .items
            .iter()
            .find(|i| i.rule.as_ref().is_some_and(|r| r.id == "user_temp"))
            .unwrap();
        assert_eq!(temp.bytes, 1800);
        assert_eq!(
            temp.eligible_bytes, 1500,
            "свіжий файл (1 день) не підпадає під 7 днів"
        );
        assert!(temp.selectable);
        let chrome: Vec<_> = report
            .items
            .iter()
            .filter(|i| i.rule.as_ref().is_some_and(|r| r.id == "chrome_cache"))
            .collect();
        assert_eq!(chrome.len(), 2, "обидва профілі");
        assert!(
            chrome.iter().all(|i| i.bytes < 999),
            "History не належить кешу"
        );
        let winsxs = report
            .items
            .iter()
            .find(|i| i.path.ends_with("WinSxS"))
            .unwrap();
        assert_eq!(winsxs.verdict, Verdict::Protected);
        assert!(!winsxs.selectable);
    }

    #[test]
    fn walk_does_not_follow_junction() {
        let fs = sample();
        let rs = rules();
        let idx = rs.index_of("user_temp").unwrap();
        let root = r"C:\Users\Test\AppData\Local\Temp";
        let meta = fs.metadata(root).unwrap();
        let walk = walk_root(
            &fs,
            rs.get(idx).unwrap(),
            root,
            &meta,
            NOW,
            &CancelToken::new(),
        );
        assert_eq!(walk.reparse_skipped, 1);
        assert!(walk.eligible.iter().all(|c| !c.path.contains("evil")));
        assert!(walk.eligible.iter().all(|c| !c.path.contains("System32")));
        assert_eq!(walk.dirs, vec![format!(r"{root}\sub")]);
    }

    #[test]
    fn full_scan_reports_unknown_big_folders() {
        let fs = sample();
        let report = scan(
            &fs,
            &rules(),
            &opts(true),
            &CancelToken::new(),
            &|_| {},
            &|_| {},
        );
        let big = report
            .unknown
            .iter()
            .find(|i| i.path == r"C:\Games\Big")
            .unwrap();
        assert_eq!(big.verdict, Verdict::Unknown);
        assert!(!big.selectable);
        assert!(report.scanned_bytes >= 2 * GIB);
        assert!(report.sizes.iter().any(|(p, _)| p == r"C:\Games"));
    }

    #[test]
    fn keep_latest_keeps_newest_children() {
        let text = r#"
[[rule]]
id = "drv"
category = "drivers"
paths = ['%SystemDrive%\Drv']
description = "x"
verdict = "safe"
regenerates = false
consequences = "x"
method = "delete"
keep_latest = 1
source = "test"
verified_on = "test"
"#;
        let (rs, issues) = RuleSet::from_sources(Env::for_tests(), &[("t", text)]);
        assert!(issues.is_empty(), "{issues:?}");
        let fs = MemFs::new();
        fs.add_file(r"C:\Drv\560.00\a.bin", 10, 0)
            .add_file(r"C:\Drv\572.16\a.bin", 20, 0);
        fs.add_dir_at(r"C:\Drv\560.00", NOW - 90 * DAY)
            .add_dir_at(r"C:\Drv\572.16", NOW - DAY);
        let rule = rs.get(0).unwrap();
        let meta = fs.metadata(r"C:\Drv").unwrap();
        let walk = walk_root(&fs, rule, r"C:\Drv", &meta, NOW, &CancelToken::new());
        assert_eq!(walk.bytes, 30);
        let paths: Vec<_> = walk.eligible.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![r"C:\Drv\560.00\a.bin"],
            "найновіша збірка лишається"
        );
    }

    /// Дерево з кількома рівнями й кандидатами в різних гілках.
    fn wide_temp(fs: &MemFs) -> &'static str {
        let temp = r"C:\Users\Test\AppData\Local\Temp";
        for a in 0..12 {
            for b in 0..5 {
                let age = if (a + b) % 3 == 0 { DAY } else { 30 * DAY };
                fs.add_file(
                    &format!(r"{temp}\a{a}\b{b}\f.tmp"),
                    10 + a * 7 + b,
                    NOW - age,
                );
                fs.add_file(&format!(r"{temp}\a{a}\b{b}\deep\g.tmp"), 3, NOW - 30 * DAY);
            }
            fs.add_file(&format!(r"{temp}\a{a}\top.tmp"), 1, NOW - 30 * DAY);
        }
        fs.add_link(&format!(r"{temp}\a3\evil"), r"C:\Windows\System32", true);
        temp
    }

    #[test]
    fn parallel_walk_matches_single_thread() {
        let fs = MemFs::new();
        let temp = wide_temp(&fs);
        let rs = rules();
        let rule = rs.get(rs.index_of("user_temp").unwrap()).unwrap();
        let meta = fs.metadata(temp).unwrap();
        let one = walk_root(&fs, rule, temp, &meta, NOW, &CancelToken::new());
        for threads in [2, 4, 8] {
            let many = walk_root_with(
                &fs,
                rule,
                temp,
                &meta,
                NOW,
                threads,
                &CancelToken::new(),
                &|_, _| {},
            );
            assert_eq!(many.bytes, one.bytes, "{threads} потоків");
            assert_eq!(many.files, one.files);
            assert_eq!(many.eligible, one.eligible);
            assert_eq!(many.dirs, one.dirs);
            assert_eq!(many.reparse_skipped, 1);
        }
        assert!(one.eligible.iter().all(|c| !c.path.contains("evil")));
        // Каталоги — від найглибших до верхніх.
        let depths: Vec<_> = one.dirs.iter().map(|d| path::depth(d)).collect();
        assert!(depths.windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn cancelled_parallel_walk_stops() {
        let fs = MemFs::new();
        let temp = wide_temp(&fs);
        let rs = rules();
        let rule = rs.get(rs.index_of("user_temp").unwrap()).unwrap();
        let meta = fs.metadata(temp).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let walk = walk_root_with(&fs, rule, temp, &meta, NOW, 4, &cancel, &|_, _| {});
        assert_eq!(walk.files, 0);
        assert!(walk.eligible.is_empty());
    }

    #[test]
    fn protected_roots_go_last_and_partial_reports_arrive() {
        let fs = sample();
        let order = Mutex::new(Vec::<String>::new());
        let partials = Mutex::new(Vec::<Vec<Verdict>>::new());
        let progress = |p: ScanProgress| {
            let mut o = order.lock().unwrap();
            if o.last() != Some(&p.current) {
                o.push(p.current);
            }
        };
        let partial = |r: &ScanReport| {
            partials
                .lock()
                .unwrap()
                .push(r.items.iter().map(|i| i.verdict).collect())
        };
        let report = scan(
            &fs,
            &rules(),
            &opts(false),
            &CancelToken::new(),
            &progress,
            &partial,
        );
        let order = order.into_inner().unwrap();
        let winsxs = order.iter().position(|p| p.ends_with("WinSxS")).unwrap();
        let temp = order
            .iter()
            .position(|p| p.ends_with(r"Local\Temp"))
            .unwrap();
        assert!(
            temp < winsxs,
            "🔴 обходиться після того, що можна чистити: {order:?}"
        );
        // Прогрес кореня приходить до його обходу: WinSxS з'являється в прогресі, хоча після нього
        // більше нічого немає.
        assert_eq!(order.last().map(String::as_str), Some(r"C:\Windows\WinSxS"));
        let partials = partials.into_inner().unwrap();
        assert_eq!(partials.len(), 1, "за правилами — один проміжний звіт");
        assert!(partials[0].iter().all(|v| *v != Verdict::Protected));
        assert!(report.items.iter().any(|i| i.verdict == Verdict::Protected));

        let count = Mutex::new(0);
        scan(
            &fs,
            &rules(),
            &opts(true),
            &CancelToken::new(),
            &|_| {},
            &|_| {
                *count.lock().unwrap() += 1;
            },
        );
        assert_eq!(
            *count.lock().unwrap(),
            2,
            "повний скан — ще один перед обходом диска"
        );
    }

    #[test]
    fn nothing_eligible_when_all_files_fresh() {
        let fs = MemFs::new();
        fs.add_file(
            r"C:\Users\Test\AppData\Local\Temp\fresh.tmp",
            300,
            NOW - DAY,
        )
        .add_file(r"C:\Windows\WinSxS\x\comp.dll", 300, NOW);
        let report = scan(
            &fs,
            &rules(),
            &opts(false),
            &CancelToken::new(),
            &|_| {},
            &|_| {},
        );
        let temp = report
            .items
            .iter()
            .find(|i| i.rule.as_ref().is_some_and(|r| r.id == "user_temp"))
            .unwrap();
        assert!(!temp.selectable);
        assert!(temp.nothing_eligible);
        assert_eq!(temp.rule.as_ref().unwrap().min_age_days, Some(7));
        let winsxs = report
            .items
            .iter()
            .find(|i| i.path.ends_with("WinSxS"))
            .unwrap();
        assert!(
            !winsxs.nothing_eligible,
            "🔴 не «чекає» — його не можна вибрати взагалі"
        );
    }

    #[test]
    fn full_scan_splits_zones_and_lists_small_subdirs() {
        const MIB: u64 = 1024 * 1024;
        let fs = sample();
        fs.add_file(r"C:\Program Files\Game\data.pak", GIB, NOW)
            .add_file(r"C:\Data\Proj\Big\x.bin", 600 * MIB, NOW)
            .add_file(r"C:\Data\Proj\s1\x.bin", 300 * MIB, NOW)
            .add_file(r"C:\Data\Proj\s2\x.bin", 200 * MIB, NOW)
            .add_file(r"C:\Data\Proj\s3\x.bin", 100 * MIB, NOW)
            .add_file(r"C:\Data\Proj\loose.bin", 10 * MIB, NOW);
        let report = scan(
            &fs,
            &rules(),
            &opts(true),
            &CancelToken::new(),
            &|_| {},
            &|_| {},
        );
        assert!(report.unknown.iter().all(|i| i.verdict == Verdict::Unknown));
        assert!(report.unknown.iter().all(|i| i.zone.is_none()));
        let game = report
            .protected_zones
            .iter()
            .find(|i| i.path == r"C:\Program Files\Game")
            .unwrap();
        assert_eq!(game.verdict, Verdict::Protected);
        assert!(!report
            .unknown
            .iter()
            .any(|i| i.path.starts_with(r"C:\Program Files")));
        let rest = report
            .unknown
            .iter()
            .find(|i| i.path == r"C:\Data\Proj")
            .unwrap();
        assert!(rest.remainder);
        assert_eq!(rest.bytes, 610 * MIB);
        assert_eq!(rest.small_dirs, 3);
        let subs: Vec<_> = rest
            .top_subdirs
            .iter()
            .map(|d| (d.path.as_str(), d.bytes))
            .collect();
        assert_eq!(
            subs,
            vec![
                (r"C:\Data\Proj\s1", 300 * MIB),
                (r"C:\Data\Proj\s2", 200 * MIB),
                (r"C:\Data\Proj\s3", 100 * MIB),
            ],
            "найбільші спершу, велика Big — окремим рядком"
        );
        assert!(report.unknown.iter().any(|i| i.path == r"C:\Data\Proj\Big"));
    }
}
