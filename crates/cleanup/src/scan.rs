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
/// Скільки невідомих тек показувати.
pub const UNKNOWN_LIMIT: usize = 30;
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
        }
    }
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
    /// Правило (немає — невідоме).
    pub rule: Option<RuleInfo>,
    /// Зона чорного списку.
    pub zone: Option<Zone>,
    /// Розмір — лише файли самої теки та дрібних підтек (великі підтеки показано окремо).
    pub remainder: bool,
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
    /// Великі теки, яких база не знає (лише повний скан).
    pub unknown: Vec<Item>,
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

/// Обходить корінь правила (каталог або файл), не заходячи в reparse points, і відбирає файли
/// за умовами правила. Нічого не змінює.
pub fn walk_root(
    fs: &dyn FsApi,
    rule: &LoadedRule,
    root: &str,
    root_meta: &Meta,
    now_unix: i64,
    cancel: &CancelToken,
) -> RootWalk {
    let mut out = RootWalk::default();
    let r = &rule.rule;
    let old_enough = |m: &Meta| match r.min_age_days {
        Some(days) => now_unix - m.newest_unix >= i64::from(days) * 86_400,
        None => true,
    };
    let name_ok = |name: &str| {
        r.file_patterns.is_empty() || r.file_patterns.iter().any(|p| path::glob_name(p, name))
    };
    // Для 🔴 / ⚪ і штатних способів список файлів не потрібен — лише розмір.
    let collect = rule.deletable();
    if root_meta.kind == Kind::File {
        out.bytes = root_meta.len;
        out.files = 1;
        if collect && old_enough(root_meta) && name_ok(path::file_name(root)) {
            out.eligible.push(Candidate {
                path: root.to_string(),
                bytes: root_meta.len,
            });
        }
        return out;
    }
    // «Залишити N найновіших» — серед прямих нащадків кореня.
    let mut kept: Vec<String> = Vec::new();
    let top = match fs.read_dir(root) {
        Ok(t) => t,
        Err(_) => {
            out.unreadable_dirs += 1;
            return out;
        }
    };
    if let Some(n) = r.keep_latest {
        let mut by_age: Vec<_> = top.iter().filter(|e| !e.meta.reparse).collect();
        by_age.sort_by_key(|e| std::cmp::Reverse(e.meta.newest_unix));
        kept = by_age
            .iter()
            .take(usize::try_from(n).unwrap_or(usize::MAX))
            .map(|e| path::key(&path::join(root, &e.name)))
            .collect();
    }
    // Обхід у глибину з явним стеком: (каталог, вміст уже прочитано?, під «залишити»?).
    let mut stack: Vec<(String, Option<Vec<crate::fs::Entry>>, bool)> =
        vec![(root.to_string(), Some(top), false)];
    let mut dirs_with_candidates: Vec<String> = Vec::new();
    while let Some((dir, entries, keep)) = stack.pop() {
        if cancel.is_cancelled() {
            break;
        }
        let entries = match entries {
            Some(e) => e,
            None => match fs.read_dir(&dir) {
                Ok(e) => e,
                Err(_) => {
                    out.unreadable_dirs += 1;
                    continue;
                }
            },
        };
        let mut had_candidate = false;
        for e in entries {
            let p = path::join(&dir, &e.name);
            if e.meta.reparse {
                out.reparse_skipped += 1;
                continue;
            }
            let keep_this = keep || (!kept.is_empty() && kept.contains(&path::key(&p)));
            match e.meta.kind {
                Kind::Dir => stack.push((p, None, keep_this)),
                Kind::File => {
                    out.bytes += e.meta.len;
                    out.files += 1;
                    if collect && !keep_this && old_enough(&e.meta) && name_ok(&e.name) {
                        out.eligible.push(Candidate {
                            path: p,
                            bytes: e.meta.len,
                        });
                        had_candidate = true;
                    }
                }
            }
        }
        if had_candidate && !path::eq_ci(&dir, root) {
            dirs_with_candidates.push(dir);
        }
    }
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
    all.sort_by_key(|d| std::cmp::Reverse(path::depth(d)));
    out.dirs = all;
    out
}

/// Скан диска за правилами (і, за потреби, повний обхід).
pub fn scan(
    fs: &dyn FsApi,
    rules: &RuleSet,
    opts: &ScanOptions,
    cancel: &CancelToken,
    progress: &(dyn Fn(ScanProgress) + Sync),
) -> ScanReport {
    let started = Instant::now();
    let mut report = ScanReport {
        drive: opts.drive.clone(),
        full: opts.full,
        started_unix: opts.now_unix,
        duration_ms: 0,
        items: Vec::new(),
        unknown: Vec::new(),
        scanned_bytes: 0,
        unreadable_dirs: 0,
        cancelled: false,
        sizes: Vec::new(),
    };
    let (mut dirs, mut files, mut bytes) = (0u64, 0u64, 0u64);
    for lr in rules.rules() {
        for pat in lr
            .patterns
            .iter()
            .filter(|p| path::eq_ci(p.drive(), &opts.drive))
        {
            for (root, meta) in expand_roots(fs, pat) {
                if cancel.is_cancelled() {
                    break;
                }
                // Корінь може збігтися зі ще суворішим правилом — тоді показуємо суворіший вердикт.
                let cls = rules.classify(&root);
                let walk = walk_root(fs, lr, &root, &meta, opts.now_unix, cancel);
                dirs += 1;
                files += walk.files;
                bytes += walk.bytes;
                progress(ScanProgress {
                    phase: ScanPhase::Rules,
                    dirs,
                    files,
                    bytes,
                    current: root.clone(),
                });
                if walk.files == 0 && walk.unreadable_dirs == 0 {
                    continue;
                }
                let verdict = cls.verdict.max(lr.verdict());
                let eligible_bytes = walk.eligible.iter().map(|c| c.bytes).sum();
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
                    selectable: lr.deletable() && verdict == lr.verdict() && eligible_bytes > 0,
                    rule: Some(RuleInfo::of(lr)),
                    zone: cls.zone,
                    unreadable_dirs: walk.unreadable_dirs,
                    remainder: false,
                });
            }
        }
    }
    report.items.sort_by_key(|i| std::cmp::Reverse(i.bytes));
    if opts.full && !cancel.is_cancelled() {
        let tree = walk_drive(fs, rules, opts, cancel, progress);
        report.scanned_bytes = tree.total(0);
        report.unreadable_dirs += tree.unreadable;
        report.unknown = tree.unknown(rules);
        report.sizes = tree.snapshot_sizes();
    }
    report
        .sizes
        .extend(report.items.iter().map(|i| (i.path.clone(), i.bytes)));
    report.cancelled = cancel.is_cancelled();
    report.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
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
    fn unknown(&self, rules: &RuleSet) -> Vec<Item> {
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
        found
            .into_iter()
            .take(UNKNOWN_LIMIT)
            .map(|(i, bytes, remainder)| {
                let p = self.path(i).to_string();
                let cls = rules.classify(&p);
                Item {
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
                    rule: None,
                    zone: cls.zone,
                    unreadable_dirs: 0,
                    remainder,
                }
            })
            .collect()
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
        let report = scan(&fs, &rules(), &opts(false), &CancelToken::new(), &|_| {});
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
        let report = scan(&fs, &rules(), &opts(true), &CancelToken::new(), &|_| {});
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
}
