//! Оверлей у грі: FPS, 1% low, час кадру, CPU і GPU гри у фокусі. Вмикається налаштуванням
//! «Оверлей у грі» (за замовчуванням вимкнено), `Ctrl+Shift+F12` ховає / показує його.
//! Власний потік: вікно оверлею ([`optimax_core::overlay`]) і живі дані
//! ([`optimax_frametime::live`], окрема ETW-сесія PresentMon). Нічого в системі не змінює.
//! Першим рядком — позначка запису (діагностика, A/B, тест роздільності) або стеження
//! «Щойно лагнуло», щоб із гри було видно, що запис іде і чи рахується час.

use crate::state::{lock, AppState};
use optimax_core::input::{FunctionKey, GlobalHotkey};
use optimax_core::overlay::{LineTone, Overlay, OverlayLine};
use optimax_core::{Catalog, Text};
use optimax_frametime::live::{LiveMonitor, LiveStats};
use optimax_frametime::{Phase, RecordProgress};
use optimax_metrics::{GpuApi, NvmlGpu, SysinfoSource};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// Як часто оновлювати оверлей.
const REFRESH: Duration = Duration::from_millis(500);

/// Що показати позначкою над цифрами.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    /// Запис чекає, поки гра з'явиться у фокусі.
    WaitingForGame,
    /// Запис чекає Ctrl+Shift+F9.
    WaitingForHotkey,
    /// Прогрів: час ще не рахується.
    Warmup,
    /// Запис іде: зараховано / потрібно, с.
    Recording {
        /// Зараховано.
        counted_s: f64,
        /// Потрібно.
        target_s: f64,
    },
    /// Запис на паузі (гра не у фокусі або не малює).
    Paused,
    /// Стеження «Щойно лагнуло»: буфер пишеться, Ctrl+Shift+F11 зберігає.
    Watching,
}

/// Позначка з ходу запису (якщо він іде) або зі стеження.
pub fn mark(progress: Option<&RecordProgress>, watching: bool) -> Option<Mark> {
    match progress.map(|p| (p.phase, p)) {
        Some((Phase::WaitingForGame, _)) => Some(Mark::WaitingForGame),
        Some((Phase::WaitingForHotkey, _)) => Some(Mark::WaitingForHotkey),
        Some((Phase::Warmup, _)) => Some(Mark::Warmup),
        Some((Phase::Recording, p)) => Some(Mark::Recording {
            counted_s: p.counted_s,
            target_s: p.target_s,
        }),
        Some((Phase::Paused, _)) => Some(Mark::Paused),
        Some((Phase::Done, _)) | None => watching.then_some(Mark::Watching),
    }
}

fn current_mark(state: &AppState) -> Option<Mark> {
    let progress = lock(&state.record_progress).clone();
    mark(progress.as_ref(), super::lag::is_watching(state))
}

/// Запущений оверлей.
pub struct Controller {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Controller {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Вмикає або вимикає оверлей відповідно до налаштування.
pub fn apply(app: &AppHandle, enabled: bool) {
    let state = app.state::<AppState>();
    let running = lock(&state.overlay).is_some();
    if enabled == running {
        return;
    }
    if !enabled {
        stop(&state);
        tracing::info!("оверлей у грі вимкнено");
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let handle = {
        let app = app.clone();
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("optimax-overlay-ctl".into())
            .spawn(move || run(&app, &stop))
    };
    match handle {
        Ok(h) => {
            *lock(&state.overlay) = Some(Controller {
                stop,
                handle: Some(h),
            });
            tracing::info!("оверлей у грі ввімкнено (Ctrl+Shift+F12 — сховати / показати)");
        }
        Err(e) => tracing::error!("оверлей не запустився: {e}"),
    }
}

/// Зупиняє оверлей (вимкнення в налаштуваннях, вихід із програми).
pub fn stop(state: &AppState) {
    let c = lock(&state.overlay).take();
    drop(c);
}

fn run(app: &AppHandle, stop: &AtomicBool) {
    let overlay = match Overlay::new() {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("вікно оверлею не створено: {e}");
            return;
        }
    };
    let hotkey = match GlobalHotkey::ctrl_shift(FunctionKey::F12) {
        Ok(h) => Some(h),
        Err(e) => {
            tracing::warn!("Ctrl+Shift+F12 недоступна (зайнята іншою програмою?): {e}");
            None
        }
    };
    let state = app.state::<AppState>();
    let live = match state.presentmon() {
        Ok(pm) => {
            let gpu = NvmlGpu::open().map(|g| Box::new(g) as Box<dyn GpuApi>);
            LiveMonitor::start(pm, Box::new(SysinfoSource::new()), gpu)
        }
        Err(e) => Err(e),
    };
    let live = match live {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("оверлей: живі дані недоступні: {e}");
            return;
        }
    };
    let catalog = match Catalog::ukrainian() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("оверлей: тексти недоступні: {e}");
            return;
        }
    };
    let mut hidden = false;
    let mut shown = false;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(REFRESH);
        if hotkey.as_ref().is_some_and(GlobalHotkey::pressed) {
            hidden = !hidden;
        }
        if !live.is_running() {
            tracing::error!("оверлей: збір даних зупинився (PresentMon)");
            break;
        }
        match live.latest().filter(|_| !hidden) {
            Some(s) => {
                overlay.show(lines(&catalog, &s, current_mark(&state)));
                shown = true;
            }
            None if shown => {
                overlay.hide();
                shown = false;
            }
            None => {}
        }
    }
    overlay.hide();
    live.stop();
}

/// Рядок позначки (тексти — з каталогу).
fn mark_line(c: &Catalog, m: Mark) -> OverlayLine {
    let (text, tone) = match m {
        Mark::WaitingForGame => (Text::new("overlay.mark.wait_game"), LineTone::Muted),
        Mark::WaitingForHotkey => (Text::new("overlay.mark.wait_hotkey"), LineTone::Muted),
        Mark::Warmup => (Text::new("overlay.mark.warmup"), LineTone::Muted),
        Mark::Recording {
            counted_s,
            target_s,
        } => (
            Text::new("overlay.mark.recording")
                .arg("done", format!("{:.0}", counted_s.max(0.0).floor()))
                .arg("total", format!("{target_s:.0}")),
            LineTone::Danger,
        ),
        Mark::Paused => (Text::new("overlay.mark.paused"), LineTone::Muted),
        Mark::Watching => (Text::new("overlay.mark.watching"), LineTone::Danger),
    };
    OverlayLine {
        text: c.render(&text),
        tone,
    }
}

/// Рядки оверлею (тексти — з каталогу): позначка (якщо є) і чотири рядки цифр.
pub fn lines(c: &Catalog, s: &LiveStats, mark: Option<Mark>) -> Vec<OverlayLine> {
    let pct = |v: Option<f32>| v.map_or_else(|| "—".to_string(), |v| format!("{v:.0}"));
    let mut out: Vec<OverlayLine> = mark.map(|m| mark_line(c, m)).into_iter().collect();
    out.extend([
        OverlayLine::normal(
            c.render(
                &Text::new("overlay.fps")
                    .arg("fps", format!("{:.0}", s.fps))
                    .arg(
                        "low",
                        s.low_1pct_fps
                            .map_or_else(|| "—".to_string(), |v| format!("{v:.0}")),
                    ),
            ),
        ),
        OverlayLine::muted(
            c.render(&Text::new("overlay.ms").arg("ms", format!("{:.1}", s.frametime_ms))),
        ),
        OverlayLine::muted(
            c.render(
                &Text::new("overlay.cpu")
                    .arg("total", pct(s.cpu_percent))
                    .arg("core", pct(s.cpu_core_percent)),
            ),
        ),
        OverlayLine::muted(
            c.render(
                &Text::new("overlay.gpu").arg(
                    "gpu",
                    s.gpu_percent
                        .map_or_else(|| "—".to_string(), |v| v.to_string()),
                ),
            ),
        ),
    ]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats() -> LiveStats {
        LiveStats {
            app: "re9.exe".into(),
            fps: 137.4,
            frametime_ms: 7.28,
            low_1pct_fps: Some(92.2),
            cpu_percent: Some(41.3),
            cpu_core_percent: Some(88.0),
            gpu_percent: Some(97),
        }
    }

    #[test]
    fn lines_are_localized_and_rounded() {
        let c = Catalog::ukrainian().unwrap();
        let l: Vec<String> = lines(&c, &stats(), None)
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(l.len(), 4);
        assert!(l[0].contains("137") && l[0].contains("92"), "{}", l[0]);
        assert!(l[1].contains("7.3"), "{}", l[1]);
        assert!(l[2].contains("41") && l[2].contains("88"), "{}", l[2]);
        assert!(l[3].contains("97"), "{}", l[3]);
        assert!(l
            .iter()
            .all(|t| !t.contains("overlay.") && !t.contains('{')));
    }

    #[test]
    fn missing_values_show_dash() {
        let c = Catalog::ukrainian().unwrap();
        let s = LiveStats {
            low_1pct_fps: None,
            cpu_percent: None,
            cpu_core_percent: None,
            gpu_percent: None,
            ..stats()
        };
        let l: Vec<String> = lines(&c, &s, None).into_iter().map(|l| l.text).collect();
        assert!(l[0].contains('—'));
        assert!(l[2].contains('—'));
        assert!(l[3].contains('—'));
    }

    fn progress(phase: Phase) -> RecordProgress {
        RecordProgress {
            phase,
            counted_s: 12.7,
            target_s: 30.0,
            elapsed_s: 20.0,
        }
    }

    #[test]
    fn mark_follows_record_phase_and_watch() {
        assert_eq!(mark(None, false), None);
        assert_eq!(mark(None, true), Some(Mark::Watching));
        assert_eq!(
            mark(Some(&progress(Phase::Recording)), false),
            Some(Mark::Recording {
                counted_s: 12.7,
                target_s: 30.0
            })
        );
        assert_eq!(
            mark(Some(&progress(Phase::Warmup)), false),
            Some(Mark::Warmup)
        );
        assert_eq!(
            mark(Some(&progress(Phase::Paused)), false),
            Some(Mark::Paused)
        );
        assert_eq!(
            mark(Some(&progress(Phase::WaitingForHotkey)), false),
            Some(Mark::WaitingForHotkey)
        );
        assert_eq!(mark(Some(&progress(Phase::Done)), false), None);
    }

    #[test]
    fn mark_line_comes_first_with_its_tone() {
        let c = Catalog::ukrainian().unwrap();
        let rec = Mark::Recording {
            counted_s: 12.7,
            target_s: 30.0,
        };
        let l = lines(&c, &stats(), Some(rec));
        assert_eq!(l.len(), 5);
        assert_eq!(l[0].tone, LineTone::Danger);
        assert!(
            l[0].text.contains("12") && l[0].text.contains("30"),
            "{}",
            l[0].text
        );
        assert!(
            !l[0].text.contains("13"),
            "зараховане не округлюється вгору"
        );
        for m in [
            Mark::WaitingForGame,
            Mark::WaitingForHotkey,
            Mark::Warmup,
            Mark::Paused,
            Mark::Watching,
        ] {
            let t = &lines(&c, &stats(), Some(m))[0].text;
            assert!(!t.contains("overlay.") && !t.contains('{'), "{t}");
        }
        assert_eq!(
            lines(&c, &stats(), Some(Mark::Watching))[0].tone,
            LineTone::Danger
        );
        assert_eq!(
            lines(&c, &stats(), Some(Mark::Paused))[0].tone,
            LineTone::Muted
        );
    }
}
