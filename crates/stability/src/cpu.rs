//! Навантаження CPU: кожне фізичне ядро по черзі, потім усі логічні ядра разом, і так по колу.
//!
//! По одному ядро працює на найвищій частоті, де нестабільний Curve Optimizer чи розгін ламається
//! найчастіше. Разом ядра перевіряють живлення й охолодження. Завдання однакові для всіх ядер, тож
//! результати звіряються між ядрами ([`CoreChecker`]): ядро, що розійшлося з більшістю, названо.
//!
//! Програма не бачить температуру CPU без драйвера ядра, тож запобіжника температури тут немає
//! (рішення користувача, 2026-11-06): запуск лише після явної згоди, а від перегріву процесор
//! захищається сам.

use crate::check::{Digest, DIGEST_WORDS};
use crate::protocol::WorkerMessage;
use crate::worker::SEEDS;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Слів пам'яті на потік (16 МБ: завдання виходить за межі кешу ядра).
pub const BUFFER_WORDS: usize = 2 * 1024 * 1024;
/// Без результату так довго ядро вважається завислим, с (завдання триває частки секунди).
const HANG_S: f64 = 60.0;
/// Позначка «зараз працюють усі ядра».
const ALL_CORES: i64 = -1;
/// Позначка «потоки завершуються».
const FINISH: i64 = -2;

// ---- Звірка між ядрами ----

/// Порівнює результати того самого завдання на різних ядрах. Правильним вважається результат
/// більшості: ядро, що порахувало інакше, помилилося, навіть якщо воно було першим.
#[derive(Debug, Default)]
pub struct CoreChecker {
    seen: HashMap<u32, HashMap<Digest, HashMap<u32, u64>>>,
    observations: u64,
}

impl CoreChecker {
    /// Враховує результат завдання `seed`, порахований фізичним ядром `core`.
    pub fn observe(&mut self, seed: u32, core: u32, digest: &Digest) {
        self.observations += 1;
        *self
            .seen
            .entry(seed)
            .or_default()
            .entry(*digest)
            .or_default()
            .entry(core)
            .or_default() += 1;
    }

    /// Завдань виконано.
    pub fn rounds(&self) -> u64 {
        self.observations
    }

    /// Повторів, звірених з іншими результатами того самого завдання.
    pub fn checked(&self) -> u64 {
        self.observations - self.seen.len() as u64
    }

    /// Результат більшості для завдання: найбільше повторів, за рівності — більше різних ядер.
    fn majority(results: &HashMap<Digest, HashMap<u32, u64>>) -> Option<&Digest> {
        results
            .iter()
            .max_by_key(|(digest, cores)| (cores.values().sum::<u64>(), cores.len(), **digest))
            .map(|(digest, _)| digest)
    }

    /// Результатів, що розійшлися з більшістю.
    pub fn mismatches(&self) -> u64 {
        self.seen
            .values()
            .map(|results| {
                let majority = Self::majority(results);
                results
                    .iter()
                    .filter(|(digest, _)| Some(*digest) != majority)
                    .map(|(_, cores)| cores.values().sum::<u64>())
                    .sum::<u64>()
            })
            .sum()
    }

    /// Ядра, що розійшлися з більшістю (за зростанням).
    pub fn bad_cores(&self) -> Vec<u32> {
        let mut bad = BTreeSet::new();
        for results in self.seen.values() {
            let majority = Self::majority(results);
            for (digest, cores) in results {
                if Some(digest) != majority {
                    bad.extend(cores.keys().copied());
                }
            }
        }
        bad.into_iter().collect()
    }
}

// ---- Розклад ----

/// Тривалість фаз: скільки секунд на ядро й скільки на всі ядра разом. Одне коло вміщується в
/// тест навіть на процесорі з багатьма ядрами.
pub fn plan(cores: usize, seconds: f64) -> (f64, f64) {
    let per_core = (seconds * 2.0 / 3.0 / cores.max(1) as f64).clamp(1.0, 10.0);
    let all = (seconds / 3.0).clamp(1.0, 30.0);
    (per_core, all)
}

/// Яка фаза в момент `t`: `Some(ядро)` чи `None`, якщо працюють усі ядра.
pub fn phase_at(t: f64, cores: usize, per_core: f64, all: f64) -> Option<u32> {
    let single = per_core * cores as f64;
    let at = t % (single + all);
    (at < single).then(|| (at / per_core) as u32)
}

// ---- Завдання ----

fn mix(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// Цілі числа й пам'ять: заповнення буфера, випадкові читання й записи.
fn memory_kernel(seed: u64, buffer: &mut [u64]) -> u64 {
    let mut x = seed | 1;
    for word in buffer.iter_mut() {
        x = mix(x);
        *word = x;
    }
    let len = buffer.len() as u64;
    let mut hash = 0u64;
    for _ in 0..buffer.len() {
        x = mix(x);
        let i = (x % len) as usize;
        let value = mix(buffer[i] ^ x);
        buffer[i] = value;
        hash = hash.rotate_left(7) ^ value;
    }
    buffer
        .iter()
        .fold(hash, |h, w| h.rotate_left(1).wrapping_add(*w))
}

/// Дробові числа без векторних інструкцій.
fn scalar_kernel(seed: u64, iterations: u32) -> u64 {
    let mut f = (seed & 0xFFFF) as f64 / 65536.0 + 0.5;
    for k in 0..iterations {
        f = ((f * f + 1.0).sqrt() * std::f64::consts::PI
            + (f * std::f64::consts::TAU).sin() * 0.5
            + f64::from(k))
        .fract();
    }
    f.to_bits()
}

/// Векторні AVX2/FMA: найбільше навантаження на блоки дробових обчислень.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn vector_kernel(seed: u64, iterations: u32) -> [u64; 4] {
    use std::arch::x86_64::{
        _mm256_castpd_si256, _mm256_extract_epi64, _mm256_floor_pd, _mm256_fmadd_pd,
        _mm256_fnmadd_pd, _mm256_set1_pd, _mm256_set_pd, _mm256_sub_pd, _mm256_xor_pd,
    };
    let start = (seed & 0xFFFF) as f64 / 65536.0;
    let mut a = _mm256_set_pd(start + 0.1, start + 0.2, start + 0.3, start + 0.4);
    let mut b = _mm256_set_pd(start + 0.5, start + 0.6, start + 0.7, start + 0.8);
    let mut c = _mm256_set_pd(start + 0.9, start + 0.15, start + 0.25, start + 0.35);
    let mut d = _mm256_set_pd(start + 0.45, start + 0.55, start + 0.65, start + 0.75);
    let scale = _mm256_set1_pd(1.618_033_988_75);
    for _ in 0..iterations {
        a = _mm256_fmadd_pd(a, scale, b);
        b = _mm256_fnmadd_pd(b, scale, c);
        c = _mm256_fmadd_pd(c, scale, d);
        d = _mm256_fnmadd_pd(d, scale, a);
        // Значення лишаються в [0, 1), тож цикл не йде в нескінченність.
        a = _mm256_sub_pd(a, _mm256_floor_pd(a));
        b = _mm256_sub_pd(b, _mm256_floor_pd(b));
        c = _mm256_sub_pd(c, _mm256_floor_pd(c));
        d = _mm256_sub_pd(d, _mm256_floor_pd(d));
    }
    let sum = _mm256_castpd_si256(_mm256_xor_pd(_mm256_xor_pd(a, b), _mm256_xor_pd(c, d)));
    [
        _mm256_extract_epi64::<0>(sum) as u64,
        _mm256_extract_epi64::<1>(sum) as u64,
        _mm256_extract_epi64::<2>(sum) as u64,
        _mm256_extract_epi64::<3>(sum) as u64,
    ]
}

/// Чи є AVX2 і FMA.
pub fn has_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Одне завдання. Результат залежить лише від `seed` і `avx2`, а не від ядра.
pub fn task(seed: u32, avx2: bool, buffer: &mut [u64]) -> Digest {
    let seed = u64::from(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out: Digest = [0; DIGEST_WORDS];
    let mut hash = memory_kernel(seed, buffer);
    for (i, slot) in out.iter_mut().enumerate() {
        let part_seed = mix(seed.wrapping_add(i as u64).wrapping_add(hash));
        let scalar = scalar_kernel(part_seed, 20_000);
        #[cfg(target_arch = "x86_64")]
        let vector = if avx2 {
            // SAFETY: `avx2` істинне лише тоді, коли `has_avx2` підтвердив AVX2 і FMA.
            #[allow(unsafe_code)]
            unsafe {
                vector_kernel(part_seed, 400_000)
            }
        } else {
            [0; 4]
        };
        #[cfg(not(target_arch = "x86_64"))]
        let vector = {
            let _ = avx2;
            [0u64; 4]
        };
        hash = vector
            .iter()
            .fold(hash.rotate_left(13) ^ scalar, |h, v| mix(h ^ v));
        *slot = (hash >> 32) as u32 ^ hash as u32;
    }
    out
}

// ---- Виконання ----

/// Завдання: номер і буфер потоку → відбиток.
pub type TaskFn = Arc<dyn Fn(u32, &mut Vec<u64>) -> Digest + Send + Sync>;

/// Параметри прогону.
pub struct Stress {
    /// Тривалість, с.
    pub seconds: f64,
    /// Секунд на одне ядро.
    pub per_core_s: f64,
    /// Секунд на всі ядра разом.
    pub all_s: f64,
    /// Без результату так довго ядро вважається завислим, с.
    pub hang_s: f64,
}

/// Виконує завдання за розкладом. `cores` — логічні процесори кожного фізичного ядра; `work` рахує
/// завдання, `pin` прив'язує потік до логічного процесора. Останнє повідомлення — `Finished` або
/// `CoreHung`. Завислий потік лишається: процес однаково завершиться.
pub fn stress(
    config: &Stress,
    cores: &[Vec<usize>],
    work: TaskFn,
    pin: Arc<dyn Fn(usize) + Send + Sync>,
    stop: &AtomicBool,
    emit: &mut dyn FnMut(&WorkerMessage) -> bool,
) {
    let phase = Arc::new(AtomicI64::new(0));
    let (tx, rx) = channel::<(u32, u32, Digest)>();
    let total = u32::try_from(cores.len()).unwrap_or(u32::MAX);
    let mut handles = Vec::new();
    for (core_index, logical) in cores.iter().enumerate() {
        let core = u32::try_from(core_index).unwrap_or(u32::MAX);
        for (position, &cpu) in logical.iter().enumerate() {
            let (shared, tx, work, pin) = (
                Arc::clone(&phase),
                tx.clone(),
                Arc::clone(&work),
                Arc::clone(&pin),
            );
            let spawned = std::thread::Builder::new()
                .name(format!("stability-cpu-{cpu}"))
                .spawn(move || {
                    pin(cpu);
                    let mut buffer = Vec::new();
                    let mut next = core_index + position;
                    loop {
                        let now = shared.load(Ordering::SeqCst);
                        if now == FINISH {
                            return;
                        }
                        // Окреме ядро — лише перший логічний процесор, щоб він мав усю частоту.
                        let active = now == ALL_CORES || (now == i64::from(core) && position == 0);
                        if !active {
                            std::thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        let seed = SEEDS[next % SEEDS.len()];
                        next += 1;
                        let digest = work(seed, &mut buffer);
                        if tx.send((seed, core, digest)).is_err() {
                            return;
                        }
                    }
                });
            match spawned {
                Ok(h) => handles.push(h),
                Err(e) => {
                    emit(&WorkerMessage::Failed {
                        message: format!("не вдалося запустити потік: {e}"),
                    });
                    phase.store(FINISH, Ordering::SeqCst);
                    return;
                }
            }
        }
    }
    drop(tx);

    let started = Instant::now();
    let mut checker = CoreChecker::default();
    let mut last_report = -1.0f64;
    let mut current = phase_at(0.0, cores.len(), config.per_core_s, config.all_s);
    // Коли кожне ядро востаннє дало результат, поки воно мало працювати.
    let mut last_result = vec![0.0f64; cores.len()];
    let report = |checker: &CoreChecker,
                  current: Option<u32>,
                  elapsed: f64,
                  emit: &mut dyn FnMut(&WorkerMessage) -> bool| {
        emit(&WorkerMessage::Cores {
            current,
            bad: checker.bad_cores(),
            total,
        }) && emit(&WorkerMessage::Progress {
            elapsed_s: elapsed,
            rounds: checker.rounds(),
            checked: checker.checked(),
            mismatches: checker.mismatches(),
        })
    };
    loop {
        let elapsed = started.elapsed().as_secs_f64();
        if elapsed >= config.seconds || stop.load(Ordering::SeqCst) {
            break;
        }
        let now = phase_at(elapsed, cores.len(), config.per_core_s, config.all_s);
        if now != current {
            current = now;
            // Нова фаза: щойно активні ядра ще не могли дати результат.
            match now {
                Some(core) => {
                    if let Some(t) = last_result.get_mut(core as usize) {
                        *t = elapsed;
                    }
                }
                None => last_result.iter_mut().for_each(|t| *t = elapsed),
            }
        }
        phase.store(now.map_or(ALL_CORES, i64::from), Ordering::SeqCst);
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok((seed, core, digest)) => {
                checker.observe(seed, core, &digest);
                if let Some(t) = last_result.get_mut(core as usize) {
                    *t = elapsed;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let hung = match current {
            Some(core) => last_result
                .get(core as usize)
                .is_some_and(|t| elapsed - t > config.hang_s)
                .then_some(core),
            None => last_result
                .iter()
                .position(|t| elapsed - t > config.hang_s)
                .and_then(|i| u32::try_from(i).ok()),
        };
        if let Some(core) = hung {
            report(&checker, current, elapsed, emit);
            emit(&WorkerMessage::CoreHung { core });
            phase.store(FINISH, Ordering::SeqCst);
            return;
        }
        if elapsed - last_report >= 1.0 {
            last_report = elapsed;
            if !report(&checker, current, elapsed, emit) {
                break;
            }
        }
    }
    phase.store(FINISH, Ordering::SeqCst);
    // Потоки дораховують поточне завдання: ці результати теж звіряємо.
    let deadline = Instant::now() + Duration::from_secs(5);
    while handles.iter().any(|h| !h.is_finished()) && Instant::now() < deadline {
        while let Ok((seed, core, digest)) = rx.try_recv() {
            checker.observe(seed, core, &digest);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    while let Ok((seed, core, digest)) = rx.try_recv() {
        checker.observe(seed, core, &digest);
    }
    let elapsed = started.elapsed().as_secs_f64();
    emit(&WorkerMessage::Cores {
        current: None,
        bad: checker.bad_cores(),
        total,
    });
    emit(&WorkerMessage::Finished {
        elapsed_s: elapsed,
        rounds: checker.rounds(),
        checked: checker.checked(),
        mismatches: checker.mismatches(),
    });
}

/// Назва процесора з CPUID.
pub fn brand() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;
        let bytes: Vec<u8> = (0x8000_0002u32..=0x8000_0004)
            .flat_map(|leaf| {
                let r = __cpuid(leaf);
                [r.eax, r.ebx, r.ecx, r.edx]
            })
            .flat_map(u32::to_le_bytes)
            .take_while(|b| *b != 0)
            .collect();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        String::new()
    }
}

/// Логічні процесори кожного фізичного ядра. Лише перша група процесорів (до 64 логічних).
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn topology() -> Vec<Vec<usize>> {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };
    let fallback = || {
        let n = std::thread::available_parallelism().map_or(1, usize::from);
        (0..n).map(|i| vec![i]).collect()
    };
    let mut len = 0u32;
    // SAFETY: запит потрібного розміру без буфера; помилка «замалий буфер» тут очікувана.
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len) };
    if len == 0 {
        return fallback();
    }
    // u64: вирівнювання, достатнє для структури.
    let mut storage = vec![0u64; (len as usize).div_ceil(8)];
    let base = storage
        .as_mut_ptr()
        .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
    // SAFETY: буфер щонайменше на `len` байт і вирівняний; функція заповнює не більше `len`.
    if unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, Some(base), &mut len) }
        .is_err()
    {
        return fallback();
    }
    let mut cores = Vec::new();
    let mut offset = 0usize;
    // Записи коротші за повну структуру (об'єднання розраховане на найбільший варіант), тож межу
    // рахуємо лише за заголовком: Relationship і Size.
    while offset + 2 * size_of::<u32>() <= len as usize {
        // SAFETY: записи йдуть один за одним, `Size` кожного дає зсув наступного; `offset` у
        // межах заповненої частини буфера (умова циклу), запис вирівняний системою. Читаємо лише
        // заголовок і варіант Processor, які система заповнила для цього запису.
        let entry = unsafe {
            &*base
                .cast::<u8>()
                .add(offset)
                .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>()
        };
        if entry.Size == 0 {
            break;
        }
        // SAFETY: для RelationProcessorCore активне поле об'єднання — Processor.
        let processor = unsafe { &entry.Anonymous.Processor };
        let group = processor.GroupMask[0];
        if group.Group == 0 {
            let logical: Vec<usize> = (0..usize::BITS as usize)
                .filter(|bit| group.Mask & (1usize << bit) != 0)
                .collect();
            if !logical.is_empty() {
                cores.push(logical);
            }
        }
        offset += entry.Size as usize;
    }
    if cores.is_empty() {
        return fallback();
    }
    cores.sort();
    cores
}

/// Точка входу процесу навантаження CPU.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn run(seconds: u32, stop: &AtomicBool, emit: &mut dyn FnMut(&WorkerMessage) -> bool) {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadAffinityMask};
    let cores = topology();
    let avx2 = has_avx2();
    emit(&WorkerMessage::Started {
        adapter: brand(),
        vram_mb: 0,
        tested_mb: (BUFFER_WORDS as u64 * 8) / (1024 * 1024),
    });
    let (per_core_s, all_s) = plan(cores.len(), f64::from(seconds));
    stress(
        &Stress {
            seconds: f64::from(seconds),
            per_core_s,
            all_s,
            hang_s: HANG_S,
        },
        &cores,
        Arc::new(move |seed, buffer: &mut Vec<u64>| {
            buffer.resize(BUFFER_WORDS, 0);
            task(seed, avx2, buffer)
        }),
        Arc::new(|cpu| {
            let Some(mask) = 1usize.checked_shl(u32::try_from(cpu).unwrap_or(u32::MAX)) else {
                return;
            };
            // SAFETY: маска поточного потоку; помилка лише лишає потік неприв'язаним (тоді він
            // просто рахує не на «своєму» ядрі, а звірка між ядрами лишається чесною).
            unsafe { SetThreadAffinityMask(GetCurrentThread(), mask) };
        }),
        stop,
        emit,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(v: u32) -> Digest {
        [v; DIGEST_WORDS]
    }

    #[test]
    fn the_core_that_disagrees_with_the_majority_is_named_even_if_it_was_first() {
        let mut c = CoreChecker::default();
        // Ядро 0 порахувало першим і неправильно: «еталон» за ним назвав би винними всіх інших.
        c.observe(1, 0, &digest(9));
        for core in 1..8 {
            c.observe(1, core, &digest(5));
        }
        c.observe(2, 3, &digest(7));
        assert_eq!(c.bad_cores(), [0]);
        assert_eq!(c.mismatches(), 1);
        assert_eq!(c.rounds(), 9);
        assert_eq!(
            c.checked(),
            7,
            "перші виконання двох завдань нічого не перевіряють"
        );
    }

    #[test]
    fn clean_results_have_no_suspects() {
        let mut c = CoreChecker::default();
        for core in 0..4 {
            for seed in SEEDS {
                c.observe(seed, core, &digest(seed));
            }
        }
        assert!(c.bad_cores().is_empty());
        assert_eq!(c.mismatches(), 0);
    }

    #[test]
    fn one_round_of_the_schedule_fits_the_test() {
        let (per, all) = plan(8, 120.0);
        assert_eq!((per, all), (10.0, 30.0));
        assert!(per * 8.0 + all <= 120.0);
        let (per, all) = plan(32, 120.0);
        assert!(per * 32.0 + all <= 120.0, "{per} {all}");
    }

    #[test]
    fn schedule_visits_every_core_then_all_of_them() {
        assert_eq!(phase_at(0.0, 2, 10.0, 5.0), Some(0));
        assert_eq!(phase_at(9.9, 2, 10.0, 5.0), Some(0));
        assert_eq!(phase_at(10.0, 2, 10.0, 5.0), Some(1));
        assert_eq!(phase_at(20.0, 2, 10.0, 5.0), None);
        assert_eq!(phase_at(25.0, 2, 10.0, 5.0), Some(0), "наступне коло");
    }

    #[test]
    fn a_task_repeats_exactly_and_depends_on_the_seed() {
        let mut buffer = vec![0u64; 4096];
        for avx2 in [false, has_avx2()] {
            let a = task(SEEDS[0], avx2, &mut buffer);
            assert_eq!(a, task(SEEDS[0], avx2, &mut buffer));
            assert_ne!(a, task(SEEDS[1], avx2, &mut buffer));
            assert!(a.iter().filter(|w| **w != 0).count() >= DIGEST_WORDS - 1);
        }
    }

    #[test]
    fn a_task_gives_the_same_result_on_different_threads() {
        let reference = task(SEEDS[2], has_avx2(), &mut vec![0u64; 4096]);
        let handles: Vec<_> = (0..4)
            .map(|_| std::thread::spawn(|| task(SEEDS[2], has_avx2(), &mut vec![0u64; 4096])))
            .collect();
        for h in handles {
            assert_eq!(h.join().expect("потік"), reference);
        }
    }

    /// Прогін на підроблених завданнях: `work` дізнається про ядро через локальну змінну потоку.
    fn run_fake(cores: usize, work: TaskFn, hang_s: f64) -> Vec<WorkerMessage> {
        let layout: Vec<Vec<usize>> = (0..cores).map(|c| vec![c * 2, c * 2 + 1]).collect();
        let stop = AtomicBool::new(false);
        let mut out = Vec::new();
        stress(
            &Stress {
                seconds: 0.8,
                per_core_s: 0.1,
                all_s: 0.2,
                hang_s,
            },
            &layout,
            work,
            Arc::new(|cpu| THREAD_CPU.with(|t| t.set(cpu))),
            &stop,
            &mut |m| {
                out.push(m.clone());
                true
            },
        );
        out
    }

    std::thread_local! {
        static THREAD_CPU: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
    }

    fn finished(m: &[WorkerMessage]) -> (u64, u64) {
        match m.last() {
            Some(WorkerMessage::Finished {
                rounds, mismatches, ..
            }) => (*rounds, *mismatches),
            other => unreachable!("{other:?}"),
        }
    }

    fn last_bad(m: &[WorkerMessage]) -> Vec<u32> {
        m.iter()
            .rev()
            .find_map(|x| match x {
                WorkerMessage::Cores { bad, .. } => Some(bad.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn clean_cores_finish_without_errors_and_every_core_works() {
        let visited = Arc::new(std::sync::Mutex::new(BTreeSet::new()));
        let v = Arc::clone(&visited);
        let m = run_fake(
            3,
            Arc::new(move |seed, _| {
                if let Ok(mut set) = v.lock() {
                    set.insert(THREAD_CPU.with(std::cell::Cell::get));
                }
                std::thread::sleep(Duration::from_millis(2));
                digest(seed)
            }),
            30.0,
        );
        let (rounds, mismatches) = finished(&m);
        assert!(rounds > 0);
        assert_eq!(mismatches, 0);
        assert!(last_bad(&m).is_empty());
        let visited = visited.lock().map(|s| s.clone()).unwrap_or_default();
        // Перший логічний процесор кожного ядра (0, 2, 4) працював окремо, а другі — у фазі всіх.
        assert_eq!(visited, BTreeSet::from([0, 1, 2, 3, 4, 5]));
    }

    #[test]
    fn a_faulty_core_is_named() {
        let m = run_fake(
            4,
            Arc::new(|seed, _| {
                std::thread::sleep(Duration::from_millis(2));
                // Логічні процесори 4 і 5 — фізичне ядро 2.
                let cpu = THREAD_CPU.with(std::cell::Cell::get);
                digest(if cpu / 2 == 2 { seed ^ 1 } else { seed })
            }),
            30.0,
        );
        assert!(finished(&m).1 > 0);
        assert_eq!(last_bad(&m), [2]);
    }

    #[test]
    fn a_hung_core_ends_the_test_and_is_named() {
        let m = run_fake(
            2,
            Arc::new(|seed, _| {
                if THREAD_CPU.with(std::cell::Cell::get) / 2 == 1 {
                    std::thread::sleep(Duration::from_secs(5));
                } else {
                    std::thread::sleep(Duration::from_millis(2));
                }
                digest(seed)
            }),
            0.05,
        );
        assert_eq!(m.last(), Some(&WorkerMessage::CoreHung { core: 1 }));
    }

    #[cfg(windows)]
    #[test]
    fn topology_covers_every_logical_processor_once() {
        let cores = topology();
        let mut all: Vec<usize> = cores.iter().flatten().copied().collect();
        all.sort_unstable();
        let unique = all.len();
        all.dedup();
        assert_eq!(all.len(), unique);
        assert!(!cores.is_empty());
        assert!(!brand().is_empty());
        // Жоден логічний процесор не загубився (ПК до 64 логічних, одна група).
        let logical = std::thread::available_parallelism().map_or(0, usize::from);
        if logical <= 64 {
            assert_eq!(unique, logical, "{cores:?}");
        }
    }
}
