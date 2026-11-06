//! Навантаження оперативної пам'яті: запис і перевірка відомих даних у вільній пам'яті.
//!
//! Програма знає, що записала, тож кожне слово, що прочиталося інакше, — помилка. Потоків стільки,
//! скільки логічних процесорів, у кожного своя ділянка, тож контролер пам'яті завантажений з усіх
//! боків. Перелік перевірок: випадкові дані, що залежать від адреси, бігучі інверсії сталих зразків
//! і бігучого біта, копіювання блоками.
//!
//! Межі, про які чесно каже інтерфейс: перевіряється лише вільна пам'ять (частина, зайнята Windows
//! і програмами, тесту недоступна), а фізичних адрес програма не бачить, тож модуль чи слот не
//! названо. Сторінки закріплені в пам'яті (`VirtualLock`), щоб дані не пішли у файл підкачки: тоді
//! тест перевіряв би ще й диск.

use crate::protocol::WorkerMessage;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;
/// Менше за це перевіряти немає сенсу.
pub const MIN_TEST_BYTES: u64 = 512 * MIB;
/// Скільки вільної пам'яті лишати системі щонайменше. Припущення: щоб Windows і відкриті програми
/// не почали витісняти одне одного у файл підкачки.
const MIN_RESERVE_BYTES: u64 = 2048 * MIB;
/// Порція перевірки: 1 МіБ. Між порціями потік перевіряє зупинку й повідомляє, що живий.
const CHUNK_WORDS: usize = 128 * 1024;
/// Без жодної порції так довго потік вважається завислим, с (порція триває мілісекунди).
const HANG_S: f64 = 60.0;
/// Скільки чекати, поки потоки завершать поточну порцію, с.
const FINISH_WAIT_S: f64 = 5.0;
/// Сталі зразки бігучої інверсії: суцільні, чергування бітів і груп бітів.
const PATTERNS: [u64; 8] = [
    0,
    u64::MAX,
    0x5555_5555_5555_5555,
    0xAAAA_AAAA_AAAA_AAAA,
    0x3333_3333_3333_3333,
    0xCCCC_CCCC_CCCC_CCCC,
    0x0F0F_0F0F_0F0F_0F0F,
    0x00FF_00FF_00FF_00FF,
];
/// Перевірок за один прохід ([`Worker::pass`]).
pub const CHECKS_PER_PASS: u64 = 4;

/// Скільки байтів перевіряти: вільна пам'ять без запасу для системи (10 % обсягу, але не менше
/// 2 ГіБ). `0` — вільної пам'яті замало для тесту.
pub fn test_bytes(total: u64, available: u64) -> u64 {
    let bytes = available.saturating_sub((total / 10).max(MIN_RESERVE_BYTES));
    if bytes < MIN_TEST_BYTES {
        0
    } else {
        bytes
    }
}

/// Значення слова `index` за `seed` (перемішування SplitMix64): кожна адреса має власні дані, тож
/// дві адреси, що насправді ведуть в одне місце, дадуть розбіжність.
pub fn word(index: u64, seed: u64) -> u64 {
    let mut z = index.wrapping_add(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^ (z >> 27)
}

/// Перевірки над однією ділянкою пам'яті. Методи повертають `None`, якщо попросили зупинитись.
pub struct Worker<'a> {
    /// Зупинитись.
    pub halt: &'a AtomicBool,
    /// Лічильник порцій: доказ, що потік живий.
    pub beat: &'a AtomicU64,
    /// Слів, що прочитались не так, як записані.
    pub bad: &'a AtomicU64,
    /// Викликається після кожного запису. У тесті нічого не робить, а непрозорий виклик не дає
    /// компілятору викинути перечитування щойно записаного.
    pub after_write: &'a (dyn Fn(&mut [u64]) + Send + Sync),
}

impl Worker<'_> {
    fn alive(&self) -> Option<()> {
        if self.halt.load(Ordering::Relaxed) {
            return None;
        }
        self.beat.fetch_add(1, Ordering::Relaxed);
        Some(())
    }

    fn written(&self, mem: &mut [u64]) {
        (self.after_write)(mem);
        black_box(&mut *mem);
    }

    fn write(&self, mem: &mut [u64], value: impl Fn(u64) -> u64) -> Option<()> {
        for (c, chunk) in mem.chunks_mut(CHUNK_WORDS).enumerate() {
            self.alive()?;
            let base = (c * CHUNK_WORDS) as u64;
            for (i, w) in chunk.iter_mut().enumerate() {
                *w = value(base + i as u64);
            }
        }
        self.written(mem);
        Some(())
    }

    fn check(&self, mem: &[u64], value: impl Fn(u64) -> u64) -> Option<()> {
        for (c, chunk) in mem.chunks(CHUNK_WORDS).enumerate() {
            self.alive()?;
            let base = (c * CHUNK_WORDS) as u64;
            let diff = chunk
                .iter()
                .enumerate()
                .fold(0u64, |acc, (i, w)| acc | (*w ^ value(base + i as u64)));
            if diff != 0 {
                let wrong = chunk
                    .iter()
                    .enumerate()
                    .filter(|(i, w)| **w != value(base + *i as u64))
                    .count();
                self.bad.fetch_add(wrong as u64, Ordering::Relaxed);
            }
        }
        black_box(mem);
        Some(())
    }

    /// Випадкові дані, різні для кожної адреси.
    fn random(&self, mem: &mut [u64], seed: u64) -> Option<()> {
        self.write(mem, |i| word(i, seed))?;
        self.check(mem, |i| word(i, seed))
    }

    /// Бігуча інверсія: кожне слово читається й одразу перезаписується інверсією, вгору, а потім
    /// униз пам'яттю. Ловить слова, чий вміст псується записом у сусідні.
    fn inversion(&self, mem: &mut [u64], pattern: u64) -> Option<()> {
        self.write(mem, |_| pattern)?;
        for chunk in mem.chunks_mut(CHUNK_WORDS) {
            self.alive()?;
            let mut wrong = 0u64;
            for w in chunk.iter_mut() {
                wrong += u64::from(*w != pattern);
                *w = !pattern;
            }
            self.bad.fetch_add(wrong, Ordering::Relaxed);
        }
        self.written(mem);
        for chunk in mem.rchunks_mut(CHUNK_WORDS) {
            self.alive()?;
            let mut wrong = 0u64;
            for w in chunk.iter_mut().rev() {
                wrong += u64::from(*w != !pattern);
                *w = pattern;
            }
            self.bad.fetch_add(wrong, Ordering::Relaxed);
        }
        self.written(mem);
        self.check(mem, |_| pattern)
    }

    fn copy_half(&self, from: &[u64], to: &mut [u64]) -> Option<()> {
        for (d, s) in to.chunks_mut(CHUNK_WORDS).zip(from.chunks(CHUNK_WORDS)) {
            self.alive()?;
            d.copy_from_slice(s);
        }
        self.written(to);
        Some(())
    }

    /// Копіювання блоками між половинами ділянки в обидва боки: інший характер обміну на шині.
    fn copy(&self, mem: &mut [u64], seed: u64) -> Option<()> {
        let half = mem.len() / 2;
        let (a, b) = mem.split_at_mut(half);
        let b = &mut b[..half];
        self.write(a, |i| word(i, seed))?;
        self.copy_half(a, b)?;
        self.check(a, |i| word(i, seed))?;
        self.check(b, |i| word(i, seed))?;
        self.write(b, |i| word(i, !seed))?;
        self.copy_half(b, a)?;
        self.check(a, |i| word(i, !seed))?;
        self.check(b, |i| word(i, !seed))
    }

    /// Один прохід `n`: щоразу інші дані. `done` рахує завершені перевірки.
    pub fn pass(&self, mem: &mut [u64], n: u64, done: &AtomicU64) -> Option<()> {
        let seed = word(n, 0x7261_6D74_6573_7431);
        self.random(mem, seed)?;
        done.fetch_add(1, Ordering::Relaxed);
        self.inversion(mem, PATTERNS[(n % PATTERNS.len() as u64) as usize])?;
        done.fetch_add(1, Ordering::Relaxed);
        self.inversion(mem, 1u64 << (n % 64))?;
        done.fetch_add(1, Ordering::Relaxed);
        self.copy(mem, seed)?;
        done.fetch_add(1, Ordering::Relaxed);
        Some(())
    }
}

/// Викликається після кожного запису (у справжньому тесті нічого не робить).
pub type AfterWrite = Arc<dyn Fn(&mut [u64]) + Send + Sync>;

/// Параметри прогону.
pub struct Stress {
    /// Тривалість, с.
    pub seconds: f64,
    /// Без жодної порції так довго потік вважається завислим, с.
    pub hang_s: f64,
}

/// Перевіряє кожну ділянку у власному потоці, поки не вийде час або не попросять зупинитись.
/// Останнє повідомлення — `Finished`. Завислий потік або помилка запуску завершують тест без
/// `Finished` (`Failed` лише для запуску): програма покаже падіння процесу, а не чистий результат.
pub fn stress(
    config: &Stress,
    slices: Vec<&'static mut [u64]>,
    after_write: &AfterWrite,
    stop: &AtomicBool,
    emit: &mut dyn FnMut(&WorkerMessage) -> bool,
) {
    let halt = Arc::new(AtomicBool::new(false));
    let bad = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicU64::new(0));
    let beats: Vec<Arc<AtomicU64>> = slices.iter().map(|_| Arc::new(AtomicU64::new(0))).collect();
    let mut handles = Vec::new();
    for (index, mem) in slices.into_iter().enumerate() {
        let (halt_t, bad_t, done_t, beat_t, after_t) = (
            Arc::clone(&halt),
            Arc::clone(&bad),
            Arc::clone(&done),
            Arc::clone(&beats[index]),
            Arc::clone(after_write),
        );
        let spawned = std::thread::Builder::new()
            .name(format!("stability-ram-{index}"))
            .spawn(move || {
                let worker = Worker {
                    halt: &halt_t,
                    beat: &beat_t,
                    bad: &bad_t,
                    after_write: &*after_t,
                };
                let mut n = 0u64;
                while worker.pass(mem, n, &done_t).is_some() {
                    n += 1;
                }
            });
        match spawned {
            Ok(h) => handles.push(h),
            Err(e) => {
                halt.store(true, Ordering::SeqCst);
                emit(&WorkerMessage::Failed {
                    message: format!("не вдалося запустити потік: {e}"),
                });
                return;
            }
        }
    }

    let started = Instant::now();
    let progress = |elapsed: f64| {
        let checks = done.load(Ordering::Relaxed);
        WorkerMessage::Progress {
            elapsed_s: elapsed,
            rounds: checks,
            checked: checks,
            mismatches: bad.load(Ordering::Relaxed),
        }
    };
    let mut seen: Vec<(u64, f64)> = vec![(0, 0.0); beats.len()];
    let mut last_report = -1.0f64;
    loop {
        let elapsed = started.elapsed().as_secs_f64();
        if elapsed >= config.seconds || stop.load(Ordering::SeqCst) {
            break;
        }
        for (slot, beat) in seen.iter_mut().zip(&beats) {
            let now = beat.load(Ordering::Relaxed);
            if now != slot.0 {
                *slot = (now, elapsed);
            } else if elapsed - slot.1 > config.hang_s {
                emit(&progress(elapsed));
                halt.store(true, Ordering::SeqCst);
                return;
            }
        }
        if elapsed - last_report >= 1.0 {
            last_report = elapsed;
            if !emit(&progress(elapsed)) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    halt.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs_f64(FINISH_WAIT_S);
    while handles.iter().any(|h| !h.is_finished()) {
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let elapsed = started.elapsed().as_secs_f64();
    let checks = done.load(Ordering::Relaxed);
    emit(&WorkerMessage::Finished {
        elapsed_s: elapsed,
        rounds: checks,
        checked: checks,
        mismatches: bad.load(Ordering::Relaxed),
    });
}

/// Ділить пам'ять порівну на `threads` ділянок (залишок менший за `threads` слів не перевіряється).
pub fn split(mem: &'static mut [u64], threads: usize) -> Vec<&'static mut [u64]> {
    let threads = threads.max(1);
    let per = (mem.len() / threads).max(1);
    mem.chunks_mut(per).take(threads).collect()
}

/// Обсяг і вільна частина фізичної пам'яті, байти.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn memory() -> Option<(u64, u64)> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: u32::try_from(size_of::<MEMORYSTATUSEX>()).ok()?,
        ..Default::default()
    };
    // SAFETY: структура нашого стеку з правильно заповненим `dwLength`.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some((status.ullTotalPhys, status.ullAvailPhys))
}

/// Виділяє й закріплює пам'ять. Якщо стільки не виходить, бере вдвічі менше. Пам'ять не
/// звільняється: її забере система, коли процес тесту завершиться, тож посилання живе до кінця.
#[cfg(windows)]
#[allow(unsafe_code)]
fn allocate(
    mut bytes: u64,
    stop: &AtomicBool,
    emit: &mut dyn FnMut(&WorkerMessage) -> bool,
) -> Result<&'static mut [u64], String> {
    use windows::Win32::System::Memory::{
        VirtualAlloc, VirtualFree, VirtualLock, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE,
        PAGE_READWRITE,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    /// Порція закріплення: між порціями програма бачить, що процес живий.
    const LOCK_STEP: usize = 256 * MIB as usize;
    /// Запас робочого набору поверх перевірюваної пам'яті.
    const SLACK: usize = 64 * MIB as usize;
    let mut last_error = String::from("пам'яті замало");
    while bytes >= MIN_TEST_BYTES {
        let size = usize::try_from(bytes / MIB * MIB).map_err(|e| e.to_string())?;
        // SAFETY: псевдодескриптор поточного процесу; лише межі його робочого набору.
        let limits = unsafe {
            SetProcessWorkingSetSize(GetCurrentProcess(), size + SLACK, size + 2 * SLACK)
        };
        if let Err(e) = limits {
            last_error = format!("робочий набір на {size} байт: {e}");
            bytes /= 2;
            continue;
        }
        // SAFETY: нова область, система сама вибирає адресу.
        let base = unsafe { VirtualAlloc(None, size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        if base.is_null() {
            last_error = format!("виділення {size} байт не вдалося");
            bytes /= 2;
            continue;
        }
        let mut locked = 0usize;
        while locked < size {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let step = LOCK_STEP.min(size - locked);
            // SAFETY: діапазон лежить усередині щойно виділеної області.
            if let Err(e) = unsafe { VirtualLock(base.cast::<u8>().add(locked).cast(), step) } {
                last_error = format!("закріплення на {locked} байтах: {e}");
                break;
            }
            locked += step;
            emit(&WorkerMessage::Progress {
                elapsed_s: 0.0,
                rounds: 0,
                checked: 0,
                mismatches: 0,
            });
        }
        if locked as u64 >= MIN_TEST_BYTES || stop.load(Ordering::SeqCst) {
            // SAFETY: область виділена вище, вирівняна за сторінкою (а отже, і за `u64`), система
            // заповнила її нулями, тож кожне слово — допустиме `u64`. Посилання єдине, а область
            // не звільняється, тож `'static` чесне.
            return Ok(unsafe {
                std::slice::from_raw_parts_mut(base.cast::<u64>(), locked / size_of::<u64>())
            });
        }
        // SAFETY: звільняємо всю область, виділену вище; посилань на неї ще немає.
        let _ = unsafe { VirtualFree(base, 0, MEM_RELEASE) };
        bytes /= 2;
    }
    Err(last_error)
}

/// Точка входу процесу навантаження пам'яті.
#[cfg(windows)]
pub fn run(seconds: u32, stop: &AtomicBool, emit: &mut dyn FnMut(&WorkerMessage) -> bool) {
    let Some((total, available)) = memory() else {
        emit(&WorkerMessage::Failed {
            message: "не вдалося дізнатись обсяг пам'яті".into(),
        });
        return;
    };
    let wanted = test_bytes(total, available);
    if wanted == 0 {
        emit(&WorkerMessage::Failed {
            message: format!("замало вільної пам'яті: {} МБ", available / MIB),
        });
        return;
    }
    let mem = match allocate(wanted, stop, emit) {
        Ok(m) => m,
        Err(message) => {
            emit(&WorkerMessage::Failed { message });
            return;
        }
    };
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let slices = split(mem, threads);
    let words: usize = slices.iter().map(|s| s.len()).sum();
    emit(&WorkerMessage::Started {
        adapter: "RAM".into(),
        vram_mb: total / MIB,
        tested_mb: words as u64 * 8 / MIB,
    });
    stress(
        &Stress {
            seconds: f64::from(seconds),
            hang_s: HANG_S,
        },
        slices,
        &(Arc::new(|_: &mut [u64]| {}) as AfterWrite),
        stop,
        emit,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * MIB;

    /// Прохід по `mem`; повертає кількість хибних слів.
    fn run_pass(
        mem: &mut [u64],
        n: u64,
        after_write: &(dyn Fn(&mut [u64]) + Send + Sync),
    ) -> (u64, u64) {
        let (halt, beat, bad, done) = (
            AtomicBool::new(false),
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
        );
        let w = Worker {
            halt: &halt,
            beat: &beat,
            bad: &bad,
            after_write,
        };
        assert!(w.pass(mem, n, &done).is_some());
        (bad.into_inner(), done.into_inner())
    }

    #[test]
    fn test_size_leaves_room_for_the_system() {
        // 32 ГіБ, вільно 15: запас 3,2 ГіБ.
        assert_eq!(test_bytes(32 * GIB, 15 * GIB), 15 * GIB - 32 * GIB / 10);
        // 8 ГіБ, вільно 4: запас не менший за 2 ГіБ.
        assert_eq!(test_bytes(8 * GIB, 4 * GIB), 2 * GIB);
        // Замало вільної пам'яті.
        assert_eq!(test_bytes(16 * GIB, 2 * GIB), 0);
        assert_eq!(test_bytes(16 * GIB, 0), 0);
    }

    #[test]
    fn healthy_memory_shows_no_errors_on_many_passes() {
        let mut mem = vec![0u64; 3 * CHUNK_WORDS + 17];
        for n in 0..70 {
            let (bad, done) = run_pass(&mut mem, n, &|_| {});
            assert_eq!(bad, 0, "прохід {n}");
            assert_eq!(done, CHECKS_PER_PASS);
        }
    }

    #[test]
    fn a_single_flipped_bit_is_found() {
        let mut mem = vec![0u64; 1000];
        let flipped = AtomicBool::new(false);
        let (bad, _) = run_pass(&mut mem, 0, &|m| {
            if !flipped.swap(true, Ordering::SeqCst) {
                m[500] ^= 1 << 13;
            }
        });
        assert_eq!(bad, 1);
    }

    #[test]
    fn a_stuck_bit_is_found_whatever_value_it_is_stuck_at() {
        for stuck_high in [false, true] {
            let mut mem = vec![0u64; 1000];
            let mut total = 0;
            for n in 0..8 {
                let (bad, _) = run_pass(&mut mem, n, &|m| {
                    if stuck_high {
                        m[321] |= 1 << 40;
                    } else {
                        m[321] &= !(1 << 40);
                    }
                });
                total += bad;
            }
            assert!(total > 0, "застрягло в {stuck_high}");
        }
    }

    #[test]
    fn two_addresses_leading_to_one_place_are_found() {
        // Адресна помилка: запис за адресою 100 потрапляє ще й на 200.
        let mut mem = vec![0u64; 1000];
        let (bad, _) = run_pass(&mut mem, 0, &|m| m[200] = m[100]);
        assert!(bad > 0);
    }

    #[test]
    fn errors_are_counted_past_the_first_chunk_and_at_the_very_end() {
        let len = 2 * CHUNK_WORDS + 5;
        let mut mem = vec![0u64; len];
        let (bad, _) = run_pass(&mut mem, 0, &move |m| {
            if m.len() == len {
                let last = m.len() - 1;
                m[last] ^= 1;
                m[CHUNK_WORDS + 3] ^= 1;
            }
        });
        assert!(bad >= 2, "{bad}");
    }

    #[test]
    fn stop_request_ends_a_pass_without_counting_it() {
        let (halt, beat, bad, done) = (
            AtomicBool::new(true),
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
        );
        let w = Worker {
            halt: &halt,
            beat: &beat,
            bad: &bad,
            after_write: &|_| {},
        };
        assert!(w.pass(&mut vec![0u64; 100], 0, &done).is_none());
        assert_eq!(done.into_inner(), 0);
    }

    #[test]
    fn splitting_gives_equal_separate_parts() {
        let mem: &'static mut [u64] = vec![0u64; 1003].leak();
        let parts = split(mem, 4);
        assert_eq!(parts.len(), 4);
        assert!(parts.iter().all(|p| p.len() == 250));
        let mem: &'static mut [u64] = vec![0u64; 3].leak();
        assert!(split(mem, 8).len() <= 3);
    }

    fn run_stress(after_write: AfterWrite, seconds: f64, hang_s: f64) -> Vec<WorkerMessage> {
        let mem: &'static mut [u64] = vec![0u64; 32 * 1024].leak();
        let stop = AtomicBool::new(false);
        let mut out = Vec::new();
        stress(
            &Stress { seconds, hang_s },
            split(mem, 4),
            &after_write,
            &stop,
            &mut |m| {
                out.push(m.clone());
                true
            },
        );
        out
    }

    #[test]
    fn clean_run_finishes_with_checks_and_no_errors() {
        let m = run_stress(Arc::new(|_| {}), 0.3, 30.0);
        match m.last() {
            Some(WorkerMessage::Finished {
                checked,
                mismatches,
                ..
            }) => {
                assert!(*checked >= CHECKS_PER_PASS, "{checked}");
                assert_eq!(*mismatches, 0);
            }
            other => unreachable!("{other:?}"),
        }
    }

    #[test]
    fn faulty_memory_is_reported_by_the_whole_run() {
        let m = run_stress(Arc::new(|mem| mem[7] ^= 1), 0.3, 30.0);
        match m.last() {
            Some(WorkerMessage::Finished { mismatches, .. }) => assert!(*mismatches > 0),
            other => unreachable!("{other:?}"),
        }
    }

    #[test]
    fn a_hung_thread_ends_the_test_without_a_clean_finish() {
        let m = run_stress(
            Arc::new(|_| {
                std::thread::sleep(Duration::from_secs(3));
            }),
            2.0,
            0.2,
        );
        assert!(!m
            .iter()
            .any(|x| matches!(x, WorkerMessage::Finished { .. })));
    }

    #[cfg(windows)]
    #[test]
    fn memory_size_is_plausible() {
        let (total, available) = memory().expect("обсяг пам'яті");
        assert!(total > GIB);
        assert!(available <= total);
    }
}
