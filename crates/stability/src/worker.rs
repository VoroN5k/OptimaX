//! Процес навантаження: цикл завдань і звірка результатів. Сам цикл не залежить від заліза
//! ([`Load`]), тож перевіряється тестами на підробленому навантаженні.

use crate::check::{Checker, Digest, Observed};
use crate::protocol::WorkerMessage;
use std::cell::RefCell;

/// Фіксовані завдання, що виконуються по колу. Кілька різних, щоб різні дані проходили через
/// залізо, і не забагато, щоб кожне встигало повторитися навіть у короткому тесті.
pub const SEEDS: [u32; 4] = [0x6F70_7431, 0x6D61_7832, 0x7374_6233, 0x6C61_6234];
/// Як часто повідомляти про хід, с.
const PROGRESS_EVERY_S: f64 = 1.0;

/// Помилка навантаження.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Драйвер скинув відеокарту або вона зникла (HRESULT причини).
    DeviceLost(u32),
    /// Тест не вдалося виконати.
    Other(String),
}

/// Навантаження, що виконує завдання.
pub trait Load {
    /// Виконує завдання `seed`. `stop` викликається часто між частинами роботи; `true` — припинити
    /// (тоді `Ok(None)`: незавершене завдання не звіряється).
    fn round(
        &mut self,
        seed: u32,
        stop: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Digest>, LoadError>;
}

/// Виконує завдання по колу, поки не вийде `seconds`, не попросять зупинитись або `emit` не
/// поверне `false` (програма вже не читає). `now` — секунди від початку. Останнє повідомлення —
/// `Finished`, `DeviceLost` або `Failed`.
pub fn run_loop(
    load: &mut dyn Load,
    seconds: f64,
    now: &dyn Fn() -> f64,
    stop: &dyn Fn() -> bool,
    emit: &mut dyn FnMut(&WorkerMessage) -> bool,
) {
    let mut checker = Checker::default();
    let mut index = 0usize;
    let mut last_progress = 0.0f64;
    let mut gone = false;
    loop {
        let seed = SEEDS[index % SEEDS.len()];
        index += 1;
        let (rounds, checked, mismatches) = (checker.rounds, checker.checked, checker.mismatches);
        let outcome = {
            let state = RefCell::new((&mut *emit, &mut last_progress, &mut gone));
            let mut should_stop = || {
                let t = now();
                let mut s = state.borrow_mut();
                let (emit, last, gone) = &mut *s;
                if t - **last >= PROGRESS_EVERY_S {
                    **last = t;
                    let alive = emit(&WorkerMessage::Progress {
                        elapsed_s: t,
                        rounds,
                        checked,
                        mismatches,
                    });
                    **gone = **gone || !alive;
                }
                **gone || stop() || t >= seconds
            };
            load.round(seed, &mut should_stop)
        };
        match outcome {
            Ok(Some(digest)) => {
                if checker.observe(seed, &digest) == Observed::Mismatch {
                    gone = gone
                        || !emit(&WorkerMessage::Progress {
                            elapsed_s: now(),
                            rounds: checker.rounds,
                            checked: checker.checked,
                            mismatches: checker.mismatches,
                        });
                }
                if gone || stop() || now() >= seconds {
                    break;
                }
            }
            Ok(None) => break,
            Err(LoadError::DeviceLost(code)) => {
                emit(&WorkerMessage::DeviceLost { code });
                return;
            }
            Err(LoadError::Other(message)) => {
                emit(&WorkerMessage::Failed { message });
                return;
            }
        }
    }
    emit(&WorkerMessage::Finished {
        elapsed_s: now(),
        rounds: checker.rounds,
        checked: checker.checked,
        mismatches: checker.mismatches,
    });
}

/// Точка входу процесу навантаження: виконує тест і завершує роботу. Код виходу завжди описаний
/// повідомленням; вихід без `Finished` програма трактує як падіння.
#[cfg(windows)]
pub fn run(kind: crate::protocol::TestKind, seconds: u32) {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    let stdout = std::io::stdout();
    let mut emit = |m: &WorkerMessage| -> bool {
        let Ok(line) = serde_json::to_string(m) else {
            return true;
        };
        let mut out = stdout.lock();
        writeln!(out, "{line}").and_then(|()| out.flush()).is_ok()
    };
    let stop = Arc::new(AtomicBool::new(false));
    let on_eof = Arc::clone(&stop);
    // Закритий stdin означає «зупинись»: так само, якщо програма впала чи її закрили.
    let reader = std::thread::Builder::new().spawn(move || {
        let mut sink = [0u8; 16];
        let mut stdin = std::io::stdin();
        while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
        on_eof.store(true, Ordering::SeqCst);
    });
    if reader.is_err() {
        emit(&WorkerMessage::Failed {
            message: "не вдалося запустити потік зупинки".into(),
        });
        return;
    }
    if kind == crate::protocol::TestKind::Cpu {
        crate::cpu::run(seconds, &stop, &mut emit);
        return;
    }
    let mut load = match crate::gpu::GpuLoad::new(&crate::gpu::GpuParams::default()) {
        Ok(l) => l,
        Err(LoadError::DeviceLost(code)) => {
            emit(&WorkerMessage::DeviceLost { code });
            return;
        }
        Err(LoadError::Other(message)) => {
            emit(&WorkerMessage::Failed { message });
            return;
        }
    };
    emit(&WorkerMessage::Started {
        adapter: load.adapter().to_string(),
        vram_mb: load.vram_mb(),
        tested_mb: load.tested_mb(),
    });
    let started = Instant::now();
    run_loop(
        &mut load,
        f64::from(seconds),
        &|| started.elapsed().as_secs_f64(),
        &|| stop.load(Ordering::SeqCst),
        &mut emit,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::DIGEST_WORDS;
    use std::cell::Cell;

    /// Кожне завдання — секунда; `fail_at` — на якому завданні трапляється подія.
    struct Fake<'a> {
        clock: &'a Cell<f64>,
        done: usize,
        flip_at: Option<usize>,
        lost_at: Option<usize>,
    }

    impl Load for Fake<'_> {
        fn round(
            &mut self,
            seed: u32,
            stop: &mut dyn FnMut() -> bool,
        ) -> Result<Option<Digest>, LoadError> {
            if stop() {
                return Ok(None);
            }
            if self.lost_at == Some(self.done) {
                return Err(LoadError::DeviceLost(7));
            }
            self.clock.set(self.clock.get() + 1.0);
            let mut d = [seed; DIGEST_WORDS];
            if self.flip_at == Some(self.done) {
                d[5] ^= 1 << 7;
            }
            self.done += 1;
            Ok(Some(d))
        }
    }

    fn run_fake(
        seconds: f64,
        flip_at: Option<usize>,
        lost_at: Option<usize>,
        stop_at: f64,
    ) -> Vec<WorkerMessage> {
        let clock = Cell::new(0.0);
        let mut load = Fake {
            clock: &clock,
            done: 0,
            flip_at,
            lost_at,
        };
        let mut out = Vec::new();
        run_loop(
            &mut load,
            seconds,
            &|| clock.get(),
            &|| clock.get() >= stop_at,
            &mut |m| {
                out.push(m.clone());
                true
            },
        );
        out
    }

    fn finished(m: &[WorkerMessage]) -> (u64, u64, u64) {
        match m.last() {
            Some(WorkerMessage::Finished {
                rounds,
                checked,
                mismatches,
                ..
            }) => (*rounds, *checked, *mismatches),
            other => unreachable!("{other:?}"),
        }
    }

    #[test]
    fn clean_run_repeats_every_task_and_finds_nothing() {
        let m = run_fake(10.0, None, None, f64::MAX);
        // 10 завдань: 4 еталони, 6 повторів.
        assert_eq!(finished(&m), (10, 6, 0));
    }

    #[test]
    fn a_single_flipped_bit_is_counted_as_an_error() {
        let m = run_fake(10.0, Some(5), None, f64::MAX);
        assert_eq!(finished(&m), (10, 6, 1));
    }

    #[test]
    fn device_loss_ends_the_test_without_finished() {
        let m = run_fake(10.0, None, Some(3), f64::MAX);
        assert_eq!(m.last(), Some(&WorkerMessage::DeviceLost { code: 7 }));
        assert!(!m
            .iter()
            .any(|x| matches!(x, WorkerMessage::Finished { .. })));
    }

    #[test]
    fn stop_request_ends_the_test_early() {
        let m = run_fake(100.0, None, None, 3.0);
        assert_eq!(finished(&m), (3, 0, 0));
    }

    #[test]
    fn stops_when_the_app_no_longer_reads() {
        let clock = Cell::new(0.0);
        let mut load = Fake {
            clock: &clock,
            done: 0,
            flip_at: None,
            lost_at: None,
        };
        let mut sent = 0;
        run_loop(&mut load, 1000.0, &|| clock.get(), &|| false, &mut |_| {
            sent += 1;
            false
        });
        assert!(load.done < 5, "{}", load.done);
        assert!(sent >= 1);
    }
}
