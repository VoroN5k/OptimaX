//! Потоки гри за подіями ядра: перемикання (CSwitch), готовність (ReadyThread), потоки (Thread).
//!
//! Формати даних — класи MOF ядра з документації Microsoft («CSwitch class», «ReadyThread class»,
//! «Thread_TypeGroup1 class»). Перевірено на RE9 (2026-11-03, приклад `trace_threads`): 100 %
//! номерів потоків у перемиканнях відомі системі, 144 потоки гри.
//!
//! Для кожного потоку гри [`ThreadMachine`] ділить час на «працює / чекає на вільне ядро
//! (готовий, але не запущений) / заблокований» (з причиною очікування), рахує витіснення й
//! міграції між ядрами і розкладає час по кошиках [`BUCKET_MS`] — щоб діагностика могла взяти
//! лише зараховану гру. [`ThreadCollector`] відбирає події потоків гри в реальному часі й
//! обробляє їх по ходу запису ([`ThreadCollector::drain`]): ETW доставляє події буферами
//! окремих процесорів не по порядку, тож сортується лише те, що старше за «водяний знак».
//! Пам'ять не залежить від тривалості запису.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// GUID класу подій ядра `Thread`.
pub const THREAD_CLASS: u128 = 0x3d6fa8d1_fe05_11d0_9dda_00c04fd7ba7c;
/// Коди операцій класу `Thread`.
pub mod opcode {
    /// Потік створено.
    pub const START: u8 = 1;
    /// Потік завершився.
    pub const END: u8 = 2;
    /// Наявний потік на початку сесії.
    pub const DC_START: u8 = 3;
    /// Наявний потік у кінці сесії.
    pub const DC_END: u8 = 4;
    /// Перемикання потоків на ядрі.
    pub const CSWITCH: u8 = 36;
    /// Потік став готовим до виконання.
    pub const READY: u8 = 50;
    /// Ім'я потоку: ProcessId u32, ThreadId u32, ім'я UTF-16 з нулем.
    pub const SET_NAME: u8 = 72;
}

/// Тривалість кошика, мс.
pub const BUCKET_MS: f64 = 250.0;
/// Скільки потоків гри зберігати окремо (решта — сумою в потоці 0).
pub const KEEP_THREADS: usize = 16;
/// Для скількох найзавантаженіших потоків (плюс рендер) зберігати точні проміжки станів.
pub const DETAIL_THREADS: usize = 3;
/// Через скільки кошиків обробки вибирати потоки для точних проміжків (5 с).
const DETAIL_AFTER_BUCKETS: i64 = 20;

/// Стан потоку, що покидає ядро (`OldThreadState`, `KTHREAD_STATE`).
mod state {
    pub const READY: u8 = 1;
    pub const STANDBY: u8 = 3;
    pub const WAITING: u8 = 5;
    pub const DEFERRED_READY: u8 = 7;
}

/// Перемикання потоків на ядрі `cpu` у момент `t` (тики QPC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switch {
    /// Час, тики QPC.
    pub t: i64,
    /// Логічний процесор.
    pub cpu: u8,
    /// Потік, що починає працювати (0 — простій).
    pub new_tid: u32,
    /// Потік, що покидає ядро.
    pub old_tid: u32,
    /// Стан потоку, що покидає ядро (`KTHREAD_STATE`).
    pub old_state: u8,
    /// Причина очікування (`KWAIT_REASON`), якщо потік заблокувався.
    pub wait_reason: u8,
}

/// Подія потоку.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadEvent {
    /// Перемикання.
    Switch(Switch),
    /// Потік `tid` став готовим (його розбудили) у момент `t`.
    Ready {
        /// Час, тики QPC.
        t: i64,
        /// Потік.
        tid: u32,
    },
}

impl ThreadEvent {
    /// Час події.
    pub fn t(&self) -> i64 {
        match self {
            Self::Switch(s) => s.t,
            Self::Ready { t, .. } => *t,
        }
    }
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// CSwitch: (новий потік, старий потік, стан старого, причина очікування).
/// Розмітка: NewThreadId u32, OldThreadId u32, NewThreadPriority i8, OldThreadPriority i8,
/// PreviousCState u8, SpareByte i8, OldThreadWaitReason i8, OldThreadWaitMode i8,
/// OldThreadState i8, … — щонайменше 24 байти (у версії 5 — 28).
pub fn parse_switch(d: &[u8]) -> Option<(u32, u32, u8, u8)> {
    if d.len() < 24 {
        return None;
    }
    Some((u32_at(d, 0)?, u32_at(d, 4)?, d[14], d[12]))
}

/// ReadyThread: потік, що став готовим (TThreadId u32 + 4 байти).
pub fn parse_ready(d: &[u8]) -> Option<u32> {
    if d.len() < 8 {
        return None;
    }
    u32_at(d, 0)
}

/// Thread Start/End/DCStart/DCEnd/SetName: (процес, потік) — ProcessId u32, TThreadId u32, …
pub fn parse_thread(d: &[u8]) -> Option<(u32, u32)> {
    Some((u32_at(d, 0)?, u32_at(d, 4)?))
}

/// На що чекав заблокований потік (групи `KWAIT_REASON` з `wdm.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    /// Об'єкт синхронізації: подія, м'ютекс, інший потік, черга завершення вводу-виводу.
    Sync,
    /// Сторінка з диска (page fault).
    PageIn,
    /// Пауза (`Sleep`).
    Sleep,
    /// Інше (внутрішні очікування ядра).
    Other,
}

impl WaitKind {
    /// Група за кодом `KWAIT_REASON`.
    pub fn of(reason: u8) -> Self {
        match reason {
            // UserRequest, WrUserRequest, WrQueue, WrAlertByThreadId, Executive, WrExecutive.
            6 | 13 | 15 | 37 | 0 | 7 => Self::Sync,
            // PageIn, WrPageIn.
            2 | 9 => Self::PageIn,
            // DelayExecution, WrDelayExecution.
            4 | 11 => Self::Sleep,
            _ => Self::Other,
        }
    }

    fn slot(self) -> usize {
        match self {
            Self::Sync => slot::SYNC,
            Self::PageIn => slot::PAGE_IN,
            Self::Sleep => slot::SLEEP,
            Self::Other => slot::OTHER,
        }
    }
}

/// Індекси в кошику ([`ThreadSeries::buckets`]) і в сумах ([`ThreadSeries::totals_ms`]).
pub mod slot {
    /// Працює на ядрі.
    pub const RUNNING: usize = 0;
    /// Готовий, чекає на вільне ядро.
    pub const READY: usize = 1;
    /// Заблокований: синхронізація.
    pub const SYNC: usize = 2;
    /// Заблокований: сторінка з диска.
    pub const PAGE_IN: usize = 3;
    /// Заблокований: сон.
    pub const SLEEP: usize = 4;
    /// Заблокований: інше.
    pub const OTHER: usize = 5;
    /// Кількість.
    pub const COUNT: usize = 6;
}

/// Підсумок по одному потоку гри (у тиках QPC).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadStats {
    /// Працював на ядрі.
    pub running: i64,
    /// Готовий, але чекав на вільне ядро.
    pub ready: i64,
    /// Заблокований — за групами причин.
    pub waiting: HashMap<WaitKind, i64>,
    /// Заблокований — за сирими кодами `KWAIT_REASON` (для перевірки розмітки).
    pub wait_reasons: HashMap<u8, i64>,
    /// Скільки разів отримував ядро.
    pub runs: u32,
    /// Скільки разів продовжив на іншому ядрі, ніж минулого разу.
    pub migrations: u32,
    /// Скільки разів його витіснили (покинув ядро готовим).
    pub preempted: u32,
    /// Хто витісняв: потік → скільки разів.
    pub preemptors: HashMap<u32, u32>,
}

impl ThreadStats {
    /// Сумарно заблокований.
    pub fn waiting_total(&self) -> i64 {
        self.waiting.values().sum()
    }
}

#[derive(Debug, Clone, Copy)]
enum St {
    Running { since: i64 },
    Ready { since: i64 },
    Waiting { since: i64, reason: u8 },
}

#[derive(Debug, Clone, Default)]
struct Acc {
    stats: ThreadStats,
    buckets: Vec<[f32; slot::COUNT]>,
}

/// Покрокова машина станів потоків гри у вікні `[from, to]`. Події подаються **за порядком
/// часу** ([`ThreadMachine::feed`]); час до першої події потоку невідомий і не рахується.
#[derive(Debug, Clone)]
pub struct ThreadMachine {
    from: i64,
    to: i64,
    bucket_ticks: i64,
    tick_ms: f64,
    states: HashMap<u32, St>,
    last_cpu: HashMap<u32, u8>,
    acc: HashMap<u32, Acc>,
    /// Потоки, для яких зберігаються точні проміжки.
    detail: HashSet<u32>,
    /// Точні проміжки: (початок, кінець, [`slot`]) за порядком часу.
    intervals: HashMap<u32, Vec<(i64, i64, u8)>>,
}

impl ThreadMachine {
    /// Машина для вікна `[from, to]` (тики QPC частоти `freq`).
    pub fn new(from: i64, to: i64, freq: i64) -> Self {
        let freq = freq.max(1);
        Self {
            from,
            to,
            bucket_ticks: ((BUCKET_MS / 1000.0) * freq as f64).round().max(1.0) as i64,
            tick_ms: 1000.0 / freq as f64,
            states: HashMap::new(),
            last_cpu: HashMap::new(),
            acc: HashMap::new(),
            detail: HashSet::new(),
            intervals: HashMap::new(),
        }
    }

    /// Для цих потоків далі зберігаються точні проміжки станів.
    pub fn set_detail(&mut self, tids: impl IntoIterator<Item = u32>) {
        self.detail.extend(tids);
    }

    fn inside(&self, t: i64) -> bool {
        t >= self.from && t <= self.to
    }

    /// Додає проміжок `[a, b]` у суму й кошики потоку.
    fn add(&mut self, tid: u32, slot: usize, a: i64, b: i64) {
        let (a, b) = (a.max(self.from), b.min(self.to));
        if b <= a {
            return;
        }
        if self.detail.contains(&tid) {
            let v = self.intervals.entry(tid).or_default();
            match v.last_mut() {
                Some(last) if last.1 == a && last.2 as usize == slot => last.1 = b,
                _ => v.push((a, b, slot as u8)),
            }
        }
        let (bucket_ticks, from, tick_ms) = (self.bucket_ticks, self.from, self.tick_ms);
        let acc = self.acc.entry(tid).or_default();
        match slot {
            slot::RUNNING => acc.stats.running += b - a,
            slot::READY => acc.stats.ready += b - a,
            _ => {}
        }
        let mut t = a;
        while t < b {
            let i = usize::try_from((t - from) / bucket_ticks).unwrap_or(0);
            let end = (from + (i as i64 + 1) * bucket_ticks).min(b);
            if acc.buckets.len() <= i {
                acc.buckets.resize(i + 1, [0.0; slot::COUNT]);
            }
            acc.buckets[i][slot] += ((end - t) as f64 * tick_ms) as f32;
            t = end;
        }
    }

    fn close(&mut self, tid: u32, st: St, t: i64) {
        match st {
            St::Running { since } => self.add(tid, slot::RUNNING, since, t),
            St::Ready { since } => self.add(tid, slot::READY, since, t),
            St::Waiting { since, reason } => {
                let kind = WaitKind::of(reason);
                let d = (t.min(self.to) - since.max(self.from)).max(0);
                let s = &mut self.acc.entry(tid).or_default().stats;
                *s.waiting.entry(kind).or_default() += d;
                *s.wait_reasons.entry(reason).or_default() += d;
                self.add(tid, kind.slot(), since, t);
            }
        }
    }

    /// Подія потоку гри (`game` — множина потоків гри).
    pub fn feed(&mut self, e: &ThreadEvent, game: &HashSet<u32>) {
        match *e {
            ThreadEvent::Switch(sw) => {
                if game.contains(&sw.old_tid) {
                    if let Some(st) = self.states.remove(&sw.old_tid) {
                        self.close(sw.old_tid, st, sw.t);
                    }
                    let inside = self.inside(sw.t);
                    let next = match sw.old_state {
                        state::READY | state::DEFERRED_READY | state::STANDBY => {
                            if inside {
                                let s = &mut self.acc.entry(sw.old_tid).or_default().stats;
                                s.preempted += 1;
                                if sw.new_tid != 0 {
                                    *s.preemptors.entry(sw.new_tid).or_default() += 1;
                                }
                            }
                            Some(St::Ready { since: sw.t })
                        }
                        state::WAITING => Some(St::Waiting {
                            since: sw.t,
                            reason: sw.wait_reason,
                        }),
                        _ => None,
                    };
                    if let Some(n) = next {
                        self.states.insert(sw.old_tid, n);
                    }
                }
                if game.contains(&sw.new_tid) {
                    if let Some(st) = self.states.remove(&sw.new_tid) {
                        self.close(sw.new_tid, st, sw.t);
                    }
                    if self.inside(sw.t) {
                        let migrated = self.last_cpu.get(&sw.new_tid).is_some_and(|c| *c != sw.cpu);
                        let s = &mut self.acc.entry(sw.new_tid).or_default().stats;
                        s.runs += 1;
                        if migrated {
                            s.migrations += 1;
                        }
                    }
                    self.last_cpu.insert(sw.new_tid, sw.cpu);
                    self.states.insert(sw.new_tid, St::Running { since: sw.t });
                }
            }
            ThreadEvent::Ready { t, tid } => {
                if let Some(St::Waiting { since, reason }) = self.states.get(&tid).copied() {
                    self.close(tid, St::Waiting { since, reason }, t);
                    self.states.insert(tid, St::Ready { since: t });
                }
            }
        }
    }

    /// Закриває відкриті стани в момент `end` (не пізніше кінця вікна).
    pub fn close_all(&mut self, end: i64) {
        let end = end.min(self.to);
        let open: Vec<(u32, St)> = self.states.drain().collect();
        for (tid, st) in open {
            if self.acc.contains_key(&tid) {
                self.close(tid, st, end);
            }
        }
    }

    /// Підсумки по потоках.
    pub fn stats(&self) -> HashMap<u32, ThreadStats> {
        self.acc
            .iter()
            .map(|(tid, a)| (*tid, a.stats.clone()))
            .collect()
    }
}

/// Рахує стан потоків гри `game` у вікні `[from, to]` (події сортуються за часом).
pub fn analyze(
    events: &mut [ThreadEvent],
    game: &HashSet<u32>,
    from: i64,
    to: i64,
) -> HashMap<u32, ThreadStats> {
    events.sort_by_key(ThreadEvent::t);
    let mut m = ThreadMachine::new(from, to, 1000);
    for e in events.iter() {
        m.feed(e, game);
    }
    m.close_all(to);
    m.stats()
}

/// Ряд одного потоку гри в записі.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadSeries {
    /// Потік (0 — сума решти потоків гри).
    pub tid: u32,
    /// Ім'я потоку, якщо гра його дала.
    #[serde(default)]
    pub name: Option<String>,
    /// Скільки разів потік викликав Present (DXGI): рендер-потік.
    #[serde(default)]
    pub presents: u64,
    /// Мс у кожному кошику за станами ([`slot`]).
    pub buckets: Vec<[f32; slot::COUNT]>,
    /// Суми за весь запис, мс ([`slot`]).
    pub totals_ms: [f64; slot::COUNT],
    /// Скільки разів отримував ядро.
    pub runs: u32,
    /// Міграції між ядрами.
    pub migrations: u32,
    /// Витіснення.
    pub preempted: u32,
    /// Хто витісняв найчастіше: (процес, скільки разів).
    #[serde(default)]
    pub top_preemptor: Option<(u32, u32)>,
}

/// Потоки гри за запис.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadTrace {
    /// Частота QPC.
    pub qpc_frequency: i64,
    /// Початок кошика 0, тики QPC.
    pub start_qpc: i64,
    /// Тривалість кошика, мс.
    pub bucket_ms: f64,
    /// Потоки гри, найзавантаженіші першими ([`KEEP_THREADS`]; решта — у потоці 0).
    pub threads: Vec<ThreadSeries>,
    /// Потоків гри всього.
    pub game_threads: u32,
    /// Перемикань за участі гри.
    pub switches: u64,
    /// Подій, що прийшли запізно (після обробки свого часу) і відкинуті.
    pub late: u64,
    /// Втрачених подій ETW.
    pub lost: u64,
    /// Точні проміжки станів головних потоків — лише у вікнах навколо ривків.
    #[serde(default)]
    pub detail: Vec<ThreadDetail>,
}

/// Точні проміжки станів одного потоку.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadDetail {
    /// Потік.
    pub tid: u32,
    /// Проміжки: [початок, тривалість] у мкс від [`ThreadTrace::start_qpc`] і [`slot`].
    pub intervals: Vec<[u32; 3]>,
}

/// Збирач подій потоків гри в реальному часі (лише перемикання й пробудження, де бере участь
/// потік гри; решту відкидає одразу — повний потік перемикань важить мегабайти за секунду).
#[derive(Debug, Clone)]
pub struct ThreadCollector {
    /// Процес гри.
    pub game_pid: u32,
    /// Потоки гри.
    pub game: HashSet<u32>,
    /// Потік → процес (усі потоки системи, що трапились).
    pub owner: HashMap<u32, u32>,
    /// Імена потоків гри.
    pub names: HashMap<u32, String>,
    /// Перемикань усього.
    pub switches: u64,
    /// Перемикань, де обидва потоки відомі (перевірка розмітки).
    pub switches_known: u64,
    /// Перемикань за участі гри.
    pub switches_game: u64,
    /// Потоків гри, наявних на початку сесії.
    pub dc_game: u32,
    /// Перша й остання подія (тики QPC).
    pub span: Option<(i64, i64)>,
    /// Подій, що прийшли після обробки свого часу.
    pub late: u64,
    pending: Vec<ThreadEvent>,
    drained_to: i64,
    machine: ThreadMachine,
}

impl ThreadCollector {
    /// Збирач для процесу гри; кошики рахуються від `start_qpc` (частота `freq`).
    pub fn new(game_pid: u32, start_qpc: i64, freq: i64) -> Self {
        Self {
            game_pid,
            game: HashSet::new(),
            owner: HashMap::new(),
            names: HashMap::new(),
            switches: 0,
            switches_known: 0,
            switches_game: 0,
            dc_game: 0,
            span: None,
            late: 0,
            pending: Vec::new(),
            drained_to: i64::MIN,
            machine: ThreadMachine::new(start_qpc, i64::MAX, freq),
        }
    }

    fn push(&mut self, e: ThreadEvent) {
        if e.t() <= self.drained_to {
            self.late += 1;
        } else {
            self.pending.push(e);
        }
    }

    /// Подія класу `Thread`: код операції, час (QPC), процесор, дані.
    pub fn on_thread_event(&mut self, op: u8, t: i64, cpu: u8, d: &[u8]) {
        self.span = Some(self.span.map_or((t, t), |(a, b)| (a.min(t), b.max(t))));
        match op {
            opcode::START | opcode::DC_START => {
                if let Some((pid, tid)) = parse_thread(d) {
                    self.owner.insert(tid, pid);
                    if pid == self.game_pid && self.game.insert(tid) && op == opcode::DC_START {
                        self.dc_game += 1;
                    }
                }
            }
            opcode::SET_NAME => {
                if let Some((pid, tid)) = parse_thread(d) {
                    if pid == self.game_pid {
                        self.names
                            .insert(tid, wide_name(d.get(8..).unwrap_or_default()));
                    }
                }
            }
            opcode::CSWITCH => {
                let Some((new_tid, old_tid, old_state, wait_reason)) = parse_switch(d) else {
                    return;
                };
                self.switches += 1;
                let known = |tid: u32| tid == 0 || self.owner.contains_key(&tid);
                if known(new_tid) && known(old_tid) {
                    self.switches_known += 1;
                }
                if self.game.contains(&new_tid) || self.game.contains(&old_tid) {
                    self.switches_game += 1;
                    self.push(ThreadEvent::Switch(Switch {
                        t,
                        cpu,
                        new_tid,
                        old_tid,
                        old_state,
                        wait_reason,
                    }));
                }
            }
            opcode::READY => {
                if let Some(tid) = parse_ready(d) {
                    if self.game.contains(&tid) {
                        self.push(ThreadEvent::Ready { t, tid });
                    }
                }
            }
            _ => {}
        }
    }

    /// Обробляє події до `watermark` включно (по порядку часу); пізніші чекають.
    pub fn drain(&mut self, watermark: i64) {
        if watermark <= self.drained_to {
            return;
        }
        let (mut ready, rest): (Vec<_>, Vec<_>) =
            self.pending.drain(..).partition(|e| e.t() <= watermark);
        self.pending = rest;
        ready.sort_by_key(ThreadEvent::t);
        for e in &ready {
            self.machine.feed(e, &self.game);
        }
        self.drained_to = watermark;
    }

    /// Вибирає потоки для точних проміжків, щойно оброблено перші 5 с: [`DETAIL_THREADS`]
    /// найзавантаженіших плюс `extra` (рендер). Повертає `true`, якщо вибір зроблено.
    pub fn pick_detail(&mut self, extra: &[u32]) -> bool {
        if !self.machine.detail.is_empty() {
            return true;
        }
        let processed = self.drained_to.saturating_sub(self.machine.from);
        if processed < self.machine.bucket_ticks * DETAIL_AFTER_BUCKETS {
            return false;
        }
        let mut top: Vec<(u32, i64)> = self
            .machine
            .acc
            .iter()
            .map(|(t, a)| (*t, a.stats.running))
            .collect();
        top.sort_by_key(|(t, r)| (std::cmp::Reverse(*r), *t));
        let chosen: Vec<u32> = top
            .into_iter()
            .take(DETAIL_THREADS)
            .map(|(t, _)| t)
            .chain(extra.iter().copied().filter(|t| self.game.contains(t)))
            .collect();
        if chosen.is_empty() {
            return false;
        }
        self.machine.set_detail(chosen);
        true
    }

    /// Скільки подій чекає обробки.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Підсумки по потоках (лише вже оброблені події).
    pub fn stats(&self) -> HashMap<u32, ThreadStats> {
        self.machine.stats()
    }

    /// Обробляє все, закриває стани на кінці подій і складає підсумок запису.
    /// `presents` — виклики Present за потоками гри; `lost` — втрачені події ETW;
    /// `keep` — вікна (тики QPC), у яких зберегти точні проміжки (навколо ривків).
    pub fn finish(
        mut self,
        presents: &HashMap<u32, u64>,
        lost: u64,
        freq: i64,
        keep: &[(i64, i64)],
    ) -> ThreadTrace {
        let end = self.span.map_or(self.machine.from, |(_, b)| b);
        self.drain(i64::MAX - 1);
        self.machine.close_all(end);
        let from = self.machine.from;
        let us =
            |ticks: i64| u32::try_from(ticks.max(0) * 1_000_000 / freq.max(1)).unwrap_or(u32::MAX);
        let mut detail: Vec<ThreadDetail> = self
            .machine
            .intervals
            .drain()
            .map(|(tid, v)| {
                let intervals = v
                    .iter()
                    .flat_map(|&(a, b, slot)| {
                        keep.iter().filter_map(move |&(ka, kb)| {
                            let (s, e) = (a.max(ka), b.min(kb));
                            (e > s).then(|| [us(s - from), us(e - s), u32::from(slot)])
                        })
                    })
                    .collect();
                ThreadDetail { tid, intervals }
            })
            .filter(|d: &ThreadDetail| !d.intervals.is_empty())
            .collect();
        detail.sort_by_key(|d| d.tid);
        let mut all: Vec<(u32, Acc)> = self.machine.acc.drain().collect();
        all.sort_by_key(|(tid, a)| (std::cmp::Reverse(a.stats.running), *tid));
        let series = |tid: u32, a: &Acc, owner: &HashMap<u32, u32>| {
            let mut totals = [0.0f64; slot::COUNT];
            for b in &a.buckets {
                for (t, v) in totals.iter_mut().zip(b) {
                    *t += f64::from(*v);
                }
            }
            ThreadSeries {
                tid,
                name: self.names.get(&tid).cloned().filter(|n| !n.is_empty()),
                presents: presents.get(&tid).copied().unwrap_or(0),
                buckets: a.buckets.clone(),
                totals_ms: totals,
                runs: a.stats.runs,
                migrations: a.stats.migrations,
                preempted: a.stats.preempted,
                top_preemptor: a
                    .stats
                    .preemptors
                    .iter()
                    .filter_map(|(t, n)| owner.get(t).map(|p| (*p, *n)))
                    .fold(HashMap::<u32, u32>::new(), |mut m, (p, n)| {
                        *m.entry(p).or_default() += n;
                        m
                    })
                    .into_iter()
                    .max_by_key(|(p, n)| (*n, std::cmp::Reverse(*p))),
            }
        };
        let mut threads: Vec<ThreadSeries> = all
            .iter()
            .take(KEEP_THREADS)
            .map(|(tid, a)| series(*tid, a, &self.owner))
            .collect();
        // Решта потоків — сумою (рендер-потік, якщо поза верхівкою, усе одно окремо).
        let mut rest = Acc::default();
        for (tid, a) in all.iter().skip(KEEP_THREADS) {
            if presents.get(tid).copied().unwrap_or(0) > 0 {
                threads.push(series(*tid, a, &self.owner));
                continue;
            }
            if rest.buckets.len() < a.buckets.len() {
                rest.buckets.resize(a.buckets.len(), [0.0; slot::COUNT]);
            }
            for (r, b) in rest.buckets.iter_mut().zip(&a.buckets) {
                for (x, y) in r.iter_mut().zip(b) {
                    *x += *y;
                }
            }
            rest.stats.runs += a.stats.runs;
            rest.stats.migrations += a.stats.migrations;
            rest.stats.preempted += a.stats.preempted;
        }
        if !rest.buckets.is_empty() {
            let mut s = series(0, &rest, &HashMap::new());
            s.name = None;
            s.presents = 0;
            threads.push(s);
        }
        ThreadTrace {
            qpc_frequency: freq,
            start_qpc: self.machine.from,
            bucket_ms: BUCKET_MS,
            threads,
            game_threads: u32::try_from(self.game.len()).unwrap_or(u32::MAX),
            switches: self.switches_game,
            late: self.late,
            lost,
            detail,
        }
    }
}

fn wide_name(d: &[u8]) -> String {
    let units: Vec<u16> = d
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sw(t: i64, cpu: u8, new_tid: u32, old_tid: u32, old_state: u8, reason: u8) -> ThreadEvent {
        ThreadEvent::Switch(Switch {
            t,
            cpu,
            new_tid,
            old_tid,
            old_state,
            wait_reason: reason,
        })
    }

    #[test]
    fn parses_documented_layouts() {
        let mut cs = vec![0u8; 28];
        cs[0..4].copy_from_slice(&100u32.to_le_bytes());
        cs[4..8].copy_from_slice(&200u32.to_le_bytes());
        cs[12] = 6; // OldThreadWaitReason
        cs[14] = 5; // OldThreadState = Waiting
        assert_eq!(parse_switch(&cs), Some((100, 200, 5, 6)));
        assert_eq!(parse_switch(&cs[..20]), None);
        let mut rd = vec![0u8; 8];
        rd[0..4].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(parse_ready(&rd), Some(7));
        assert_eq!(parse_ready(&rd[..4]), None);
        let mut th = vec![0u8; 16];
        th[0..4].copy_from_slice(&38980u32.to_le_bytes());
        th[4..8].copy_from_slice(&1234u32.to_le_bytes());
        assert_eq!(parse_thread(&th), Some((38980, 1234)));
    }

    #[test]
    fn splits_time_into_running_ready_and_waiting() {
        let game: HashSet<u32> = [10].into();
        // Потік 10: працює 0–40 на ядрі 0, блокується (подія, код 6) до 70, розбуджений
        // о 70, чекає на ядро до 80 (ядро 1 — міграція), працює до 95, витіснений потоком 99
        // (покидає ядро готовим), знову на ядрі 0 о 100, працює до кінця вікна (120).
        let mut ev = vec![
            sw(80, 1, 10, 0, 0, 0),
            sw(0, 0, 10, 0, 0, 0),
            sw(40, 0, 0, 10, 5, 6),
            ThreadEvent::Ready { t: 70, tid: 10 },
            sw(95, 1, 99, 10, 1, 32),
            sw(100, 0, 10, 99, 5, 6),
        ];
        let r = analyze(&mut ev, &game, 0, 120);
        let s = &r[&10];
        assert_eq!(s.running, 40 + 15 + 20);
        assert_eq!(s.waiting.get(&WaitKind::Sync), Some(&30));
        assert_eq!(s.wait_reasons.get(&6), Some(&30));
        assert_eq!(s.ready, 10 + 5);
        assert_eq!(s.runs, 3);
        assert_eq!(s.migrations, 2); // 0 → 1 → 0
        assert_eq!(s.preempted, 1);
        assert_eq!(s.preemptors.get(&99), Some(&1));
        // Потоки не з гри не рахуються.
        assert!(!r.contains_key(&99));
        // Сума станів = вікно від першої події.
        assert_eq!(s.running + s.ready + s.waiting_total(), 120);
    }

    #[test]
    fn window_clips_intervals_and_counts() {
        let game: HashSet<u32> = [10].into();
        let mut ev = vec![sw(0, 0, 10, 0, 0, 0), sw(100, 0, 0, 10, 5, 4)];
        let r = analyze(&mut ev, &game, 50, 150);
        let s = &r[&10];
        assert_eq!(s.running, 50);
        assert_eq!(s.waiting.get(&WaitKind::Sleep), Some(&50));
        assert_eq!(s.runs, 0); // запуск до вікна
    }

    #[test]
    fn wait_groups() {
        assert_eq!(WaitKind::of(6), WaitKind::Sync);
        assert_eq!(WaitKind::of(37), WaitKind::Sync);
        assert_eq!(WaitKind::of(9), WaitKind::PageIn);
        assert_eq!(WaitKind::of(11), WaitKind::Sleep);
        assert_eq!(WaitKind::of(30), WaitKind::Other);
    }

    fn thread_data(pid: u32, tid: u32) -> Vec<u8> {
        let mut d = vec![0u8; 16];
        d[0..4].copy_from_slice(&pid.to_le_bytes());
        d[4..8].copy_from_slice(&tid.to_le_bytes());
        d
    }

    fn switch_data(new_tid: u32, old_tid: u32, old_state: u8) -> Vec<u8> {
        let mut d = vec![0u8; 24];
        d[0..4].copy_from_slice(&new_tid.to_le_bytes());
        d[4..8].copy_from_slice(&old_tid.to_le_bytes());
        d[12] = 6;
        d[14] = old_state;
        d
    }

    #[test]
    fn collector_keeps_only_game_threads_and_streams_in_order() {
        // 1 тик = 1 мс (частота 1000), кошики по 250 мс.
        let mut c = ThreadCollector::new(500, 0, 1000);
        c.on_thread_event(opcode::DC_START, 1, 0, &thread_data(500, 10));
        c.on_thread_event(opcode::DC_START, 2, 0, &thread_data(600, 20));
        let mut named = thread_data(500, 10);
        named.truncate(8);
        named.extend(
            "Render"
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes),
        );
        c.on_thread_event(opcode::SET_NAME, 3, 0, &named);
        // Події приходять не по порядку: запуск о 100 після блокування о 400.
        c.on_thread_event(opcode::CSWITCH, 400, 1, &switch_data(0, 10, 5)); // гра блокується
        c.on_thread_event(opcode::CSWITCH, 100, 1, &switch_data(10, 20, 5)); // гра запускається
        c.on_thread_event(opcode::CSWITCH, 150, 2, &switch_data(20, 0, 5)); // не гра
        c.on_thread_event(opcode::CSWITCH, 160, 2, &switch_data(777, 20, 5)); // невідомий
        assert_eq!(c.game, [10].into());
        assert_eq!(c.dc_game, 1);
        assert_eq!(c.names.get(&10).map(String::as_str), Some("Render"));
        assert_eq!((c.switches, c.switches_known, c.switches_game), (4, 3, 2));
        assert_eq!(c.pending(), 2);
        c.drain(450);
        assert_eq!(c.pending(), 0);
        // Подія, старша за оброблене, відкидається.
        c.on_thread_event(opcode::READY, 420, 1, &[10, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(c.late, 1);
        c.on_thread_event(opcode::READY, 600, 1, &[10, 0, 0, 0, 0, 0, 0, 0]);
        c.on_thread_event(opcode::CSWITCH, 700, 1, &switch_data(10, 0, 5));
        c.on_thread_event(opcode::CSWITCH, 800, 1, &switch_data(0, 10, 5));
        let presents: HashMap<u32, u64> = [(10, 5)].into();
        let t = c.finish(&presents, 3, 1000, &[]);
        assert_eq!((t.game_threads, t.switches, t.late, t.lost), (1, 4, 1, 3));
        let s = &t.threads[0];
        assert_eq!((s.tid, s.presents, s.runs), (10, 5, 2));
        assert_eq!(s.name.as_deref(), Some("Render"));
        // Працює 100–400 і 700–800; заблокований 400–600, чекає на ядро 600–700.
        assert!((s.totals_ms[slot::RUNNING] - 400.0).abs() < 1e-6);
        assert!((s.totals_ms[slot::SYNC] - 200.0).abs() < 1e-6);
        assert!((s.totals_ms[slot::READY] - 100.0).abs() < 1e-6);
        // Кошики по 250 мс: 0–250 — 150 мс роботи (100–250).
        assert!((s.buckets[0][slot::RUNNING] - 150.0).abs() < 1e-3);
        assert!((s.buckets[1][slot::RUNNING] - 150.0).abs() < 1e-3);
        assert!((s.buckets[1][slot::SYNC] - 100.0).abs() < 1e-3);
    }

    #[test]
    fn finish_keeps_top_threads_and_sums_the_rest() {
        let mut c = ThreadCollector::new(1, 0, 1000);
        let n = KEEP_THREADS as u32 + 3;
        for tid in 1..=n {
            c.on_thread_event(opcode::DC_START, 0, 0, &thread_data(1, tid));
        }
        // Потік tid працює tid мс.
        for tid in 1..=n {
            c.on_thread_event(opcode::CSWITCH, 10, 0, &switch_data(tid, 0, 5));
            c.on_thread_event(
                opcode::CSWITCH,
                10 + i64::from(tid),
                0,
                &switch_data(0, tid, 5),
            );
        }
        let presents: HashMap<u32, u64> = [(1, 9)].into(); // рендер — найменш завантажений
        let t = c.finish(&presents, 0, 1000, &[]);
        assert_eq!(t.threads[0].tid, n);
        assert_eq!(t.threads.len(), KEEP_THREADS + 2); // верхівка + рендер + сума
        assert!(t.threads.iter().any(|s| s.tid == 1 && s.presents == 9));
        let rest = t.threads.iter().find(|s| s.tid == 0).expect("сума решти");
        assert!((rest.totals_ms[slot::RUNNING] - 5.0).abs() < 1e-6); // потоки 2 і 3
    }

    #[test]
    fn detail_intervals_only_for_picked_threads_and_kept_windows() {
        // 1 тик = 1 мс. Потоки 10 (працює багато) і 20 (рендер, працює мало).
        let mut c = ThreadCollector::new(1, 0, 1000);
        c.on_thread_event(opcode::DC_START, 0, 0, &thread_data(1, 10));
        c.on_thread_event(opcode::DC_START, 0, 0, &thread_data(1, 20));
        c.on_thread_event(opcode::DC_START, 0, 0, &thread_data(1, 30));
        // Кожні 100 мс: потік 10 працює 80 мс, блокується на 20; потік 20 — 5 мс.
        for k in 0..100i64 {
            let t = k * 100;
            c.on_thread_event(opcode::CSWITCH, t, 0, &switch_data(10, 0, 5));
            c.on_thread_event(opcode::CSWITCH, t + 80, 0, &switch_data(0, 10, 5));
            c.on_thread_event(opcode::CSWITCH, t, 1, &switch_data(20, 0, 5));
            c.on_thread_event(opcode::CSWITCH, t + 5, 1, &switch_data(0, 20, 5));
            if t == 4000 {
                c.drain(t);
                // Менше 5 с оброблено — ще не вибираємо.
                assert!(!c.pick_detail(&[20]));
            }
            if t == 6000 {
                c.drain(t);
                assert!(c.pick_detail(&[20, 999]));
            }
        }
        // Вікно навколо «ривка» 7000–7300.
        let t = c.finish(&HashMap::new(), 0, 1000, &[(7000, 7300)]);
        let main = t
            .detail
            .iter()
            .find(|d| d.tid == 10)
            .expect("головний потік");
        // У вікні 7000–7300: працює 7000–7080, 7100–7180, 7200–7280 (по 80 мс = 80000 мкс).
        let running: Vec<[u32; 3]> = main
            .intervals
            .iter()
            .copied()
            .filter(|i| i[2] == slot::RUNNING as u32)
            .collect();
        assert_eq!(running.len(), 3);
        assert_eq!(running[0], [7_000_000, 80_000, slot::RUNNING as u32]);
        assert!(main
            .intervals
            .iter()
            .all(|i| i[0] >= 7_000_000 && i[0] + i[1] <= 7_300_000));
        assert!(t.detail.iter().any(|d| d.tid == 20));
        // Потік 30 нічого не робив, 999 — не з гри.
        assert!(!t.detail.iter().any(|d| d.tid == 999 || d.tid == 30));
    }
}
