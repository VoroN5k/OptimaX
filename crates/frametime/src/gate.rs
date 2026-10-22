//! «Шлюз» запису: вирішує, які відрізки часу рахуються як чиста гра.
//!
//! PresentMon і метрики пишуть увесь час, а шлюз позначає на шкалі QPC відрізки, що
//! потрапляють в аналіз. Так Alt+Tab, завантаження й меню не псують FPS і 1% low.
//! Чиста машина станів без системних викликів — усі сценарії покриті тестами.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Прогрів після першої появи гри на передньому плані (підвантаження, стабілізація частот).
pub const FIRST_WARMUP: Duration = Duration::from_secs(3);
/// Прогрів після повернення в гру (Alt+Tab сам дає кілька довгих кадрів).
pub const RESUME_WARMUP: Duration = Duration::from_secs(2);
/// Гра «малює», якщо кадри *надходили* від PresentMon не давніше за це. Порівнюємо час
/// надходження, а не час показу: PresentMon 2.x віддає кадр із затримкою ~1–2 с
/// (відстежує його до показу на екрані, ETW скидає буфери приблизно раз на секунду).
pub const PRESENTING_WINDOW: Duration = Duration::from_secs(2);

/// Як починати запис.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StartMode {
    /// Одразу після старту (для консольних інструментів і тестів).
    Immediate,
    /// Коли гра на передньому плані й малює кадри, після прогріву; поза фокусом — пауза.
    #[default]
    Auto,
    /// За глобальною гарячою клавішею Ctrl+Shift+F9 (друге натискання — стоп).
    Hotkey,
}

/// Фаза запису.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Чекаємо, поки гра з'явиться на передньому плані й почне малювати.
    WaitingForGame,
    /// Чекаємо натискання гарячої клавіші.
    WaitingForHotkey,
    /// Прогрів — час ще не рахується.
    Warmup,
    /// Рахуємо.
    Recording,
    /// Гра не в фокусі або не малює — час не рахується.
    Paused,
    /// Завершено.
    Done,
}

/// Чому запис завершився.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// Набрано потрібну тривалість.
    Completed,
    /// Зупинено гарячою клавішею.
    Hotkey,
    /// Гра закрилась або вилетіла.
    GameExited,
    /// Скасовано користувачем.
    Cancelled,
    /// Гра так і не з'явилась / клавішу не натиснули за відведений час.
    WaitTimeout,
}

/// Що відбувається зараз (знімок за один такт).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observation {
    /// Процес гри запущено.
    pub game_running: bool,
    /// Вікно гри на передньому плані.
    pub game_focused: bool,
    /// Гра нещодавно показала кадр.
    pub game_presenting: bool,
    /// Гарячу клавішу натиснули з минулого такту.
    pub hotkey: bool,
    /// Користувач скасував запис.
    pub cancel: bool,
}

/// Машина станів шлюзу.
#[derive(Debug, Clone)]
pub struct Gate {
    mode: StartMode,
    freq: i64,
    target_ticks: i64,
    max_wait_ticks: i64,
    created: i64,
    phase: Phase,
    warmup_until: i64,
    open_since: Option<i64>,
    closed_ticks: i64,
    segments: Vec<(i64, i64)>,
    seen_game: bool,
    end: Option<EndReason>,
}

fn ticks(d: Duration, freq: i64) -> i64 {
    i64::try_from(d.as_nanos() * u128::try_from(freq).unwrap_or(0) / 1_000_000_000)
        .unwrap_or(i64::MAX)
}

impl Gate {
    /// `target` — скільки чистого ігрового часу потрібно (у режимі клавіші — максимум);
    /// `max_wait` — скільки чекати старту, перш ніж здатися.
    pub fn new(mode: StartMode, target: Duration, max_wait: Duration, freq: i64, now: i64) -> Self {
        let mut g = Self {
            mode,
            freq,
            target_ticks: ticks(target, freq).max(1),
            max_wait_ticks: ticks(max_wait, freq),
            created: now,
            phase: match mode {
                StartMode::Immediate => Phase::Recording,
                StartMode::Auto => Phase::WaitingForGame,
                StartMode::Hotkey => Phase::WaitingForHotkey,
            },
            warmup_until: 0,
            open_since: None,
            closed_ticks: 0,
            segments: Vec::new(),
            seen_game: false,
            end: None,
        };
        if mode == StartMode::Immediate {
            g.open_since = Some(now);
        }
        g
    }

    /// Поточна фаза.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Причина завершення (якщо завершено).
    pub fn end_reason(&self) -> Option<EndReason> {
        self.end
    }

    /// Зараховані відрізки `[початок, кінець)` у тиках QPC.
    pub fn segments(&self) -> &[(i64, i64)] {
        &self.segments
    }

    /// Чи вже почали рахувати хоч щось.
    pub fn has_recorded(&self) -> bool {
        !self.segments.is_empty() || self.open_since.is_some()
    }

    /// Зарахований час, мс.
    pub fn counted_ms(&self, now: i64) -> f64 {
        let open = self.open_since.map_or(0, |s| now - s);
        (self.closed_ticks + open) as f64 * 1000.0 / self.freq.max(1) as f64
    }

    /// Скільки чекаємо від створення, мс.
    pub fn waited_ms(&self, now: i64) -> f64 {
        (now - self.created) as f64 * 1000.0 / self.freq.max(1) as f64
    }

    fn open(&mut self, now: i64) {
        self.open_since = Some(now);
        self.phase = Phase::Recording;
    }

    fn close(&mut self, now: i64) {
        if let Some(s) = self.open_since.take() {
            if now > s {
                self.segments.push((s, now));
                self.closed_ticks += now - s;
            }
        }
    }

    fn finish(&mut self, now: i64, reason: EndReason) {
        self.close(now);
        self.phase = Phase::Done;
        self.end = Some(reason);
    }

    fn start_warmup(&mut self, now: i64) {
        let d = if self.has_recorded() {
            RESUME_WARMUP
        } else {
            FIRST_WARMUP
        };
        self.warmup_until = now + ticks(d, self.freq);
        self.phase = Phase::Warmup;
    }

    /// Один такт (кожні ~100 мс). Повертає нову фазу.
    pub fn tick(&mut self, now: i64, obs: &Observation) -> Phase {
        if self.phase == Phase::Done {
            return Phase::Done;
        }
        if obs.game_running {
            self.seen_game = true;
        }
        if obs.cancel {
            self.finish(now, EndReason::Cancelled);
            return self.phase;
        }
        if self.seen_game && !obs.game_running {
            self.finish(now, EndReason::GameExited);
            return self.phase;
        }
        let in_game = obs.game_focused && obs.game_presenting;
        match (self.mode, self.phase) {
            (StartMode::Immediate, _) => {}
            (StartMode::Auto, Phase::WaitingForGame | Phase::Paused) if in_game => {
                self.start_warmup(now);
            }
            // Користувач сам обрав момент — без прогріву.
            (StartMode::Hotkey, Phase::WaitingForHotkey) if obs.hotkey => self.open(now),
            (StartMode::Hotkey, Phase::Paused | Phase::Warmup | Phase::Recording) if obs.hotkey => {
                self.finish(now, EndReason::Hotkey);
                return self.phase;
            }
            (StartMode::Hotkey, Phase::Paused) if in_game => self.start_warmup(now),
            (_, Phase::Warmup) => {
                if !in_game {
                    self.phase = if self.has_recorded() || self.mode == StartMode::Hotkey {
                        Phase::Paused
                    } else {
                        Phase::WaitingForGame
                    };
                } else if now >= self.warmup_until {
                    self.open(now);
                }
            }
            (_, Phase::Recording) if !in_game => {
                self.close(now);
                self.phase = Phase::Paused;
            }
            _ => {}
        }
        if self.closed_ticks + self.open_since.map_or(0, |s| now - s) >= self.target_ticks {
            // Відрізаємо рівно до цілі, щоб усі прогони мали однакову тривалість.
            let excess =
                self.closed_ticks + self.open_since.map_or(0, |s| now - s) - self.target_ticks;
            self.finish(now - excess.max(0), EndReason::Completed);
        } else if !self.has_recorded()
            && self.max_wait_ticks > 0
            && now - self.created >= self.max_wait_ticks
            && matches!(self.phase, Phase::WaitingForGame | Phase::WaitingForHotkey)
        {
            self.finish(now, EndReason::WaitTimeout);
        }
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: i64 = 1000; // 1 тик = 1 мс

    fn obs(running: bool, focused: bool, presenting: bool) -> Observation {
        Observation {
            game_running: running,
            game_focused: focused,
            game_presenting: presenting,
            ..Observation::default()
        }
    }

    /// Ганяє шлюз по 100 мс від `from` до `to` з однаковими спостереженнями.
    fn run(g: &mut Gate, from: i64, to: i64, o: Observation) -> i64 {
        let mut t = from;
        while t < to && g.phase() != Phase::Done {
            t += 100;
            g.tick(t, &o);
        }
        t
    }

    #[test]
    fn auto_waits_for_focus_then_warms_up() {
        let mut g = Gate::new(
            StartMode::Auto,
            Duration::from_secs(10),
            Duration::from_secs(600),
            F,
            0,
        );
        // 20 с Alt+Tab і завантаження: гра запущена, але не в фокусі.
        let t = run(&mut g, 0, 20_000, obs(true, false, true));
        assert_eq!(g.phase(), Phase::WaitingForGame);
        assert_eq!(g.counted_ms(t), 0.0);
        // У фокусі — спершу прогрів 3 с, без зарахування.
        let t = run(&mut g, t, t + 2_000, obs(true, true, true));
        assert_eq!(g.phase(), Phase::Warmup);
        assert_eq!(g.counted_ms(t), 0.0);
        let t = run(&mut g, t, t + 1_100, obs(true, true, true));
        assert_eq!(g.phase(), Phase::Recording);
        // Далі рівно 10 с чистої гри.
        run(&mut g, t, t + 20_000, obs(true, true, true));
        assert_eq!(g.end_reason(), Some(EndReason::Completed));
        let total: i64 = g.segments().iter().map(|(a, b)| b - a).sum();
        assert_eq!(total, 10_000);
    }

    #[test]
    fn auto_pauses_when_alt_tabbed_and_rewarms() {
        let mut g = Gate::new(
            StartMode::Auto,
            Duration::from_secs(10),
            Duration::from_secs(600),
            F,
            0,
        );
        let t = run(&mut g, 0, 3_100, obs(true, true, true)); // прогрів
        let t = run(&mut g, t, t + 4_000, obs(true, true, true)); // 4 с гри
        let before = g.counted_ms(t);
        let t = run(&mut g, t, t + 5_000, obs(true, false, true)); // Alt+Tab 5 с
        assert_eq!(g.phase(), Phase::Paused);
        // Втрата фокуса помічається на наступному такті — похибка до одного такту (100 мс).
        assert!(g.counted_ms(t) - before <= 100.0);
        // Повернення: прогрів 2 с (коротший), потім зарахування продовжується.
        let t = run(&mut g, t, t + 1_500, obs(true, true, true));
        assert_eq!(g.phase(), Phase::Warmup);
        run(&mut g, t, t + 30_000, obs(true, true, true));
        assert_eq!(g.end_reason(), Some(EndReason::Completed));
        assert_eq!(g.segments().len(), 2);
    }

    #[test]
    fn game_not_presenting_is_paused_too() {
        let mut g = Gate::new(
            StartMode::Auto,
            Duration::from_secs(10),
            Duration::from_secs(600),
            F,
            0,
        );
        let t = run(&mut g, 0, 5_000, obs(true, true, true));
        assert_eq!(g.phase(), Phase::Recording);
        // У фокусі, але кадрів немає (пауза, завантаження з чорним екраном без Present).
        run(&mut g, t, t + 1_000, obs(true, true, false));
        assert_eq!(g.phase(), Phase::Paused);
    }

    #[test]
    fn crash_ends_with_what_was_recorded() {
        let mut g = Gate::new(
            StartMode::Auto,
            Duration::from_secs(60),
            Duration::from_secs(600),
            F,
            0,
        );
        let t = run(&mut g, 0, 10_000, obs(true, true, true));
        g.tick(t + 100, &obs(false, false, false));
        assert_eq!(g.end_reason(), Some(EndReason::GameExited));
        assert!(g.counted_ms(t) > 6_000.0);
        assert_eq!(g.segments().len(), 1);
    }

    #[test]
    fn game_not_started_yet_is_not_a_crash() {
        let mut g = Gate::new(
            StartMode::Auto,
            Duration::from_secs(10),
            Duration::from_secs(600),
            F,
            0,
        );
        // Гру ще не запустили — це очікування, а не «гра вилетіла».
        run(&mut g, 0, 30_000, obs(false, false, false));
        assert_eq!(g.phase(), Phase::WaitingForGame);
    }

    #[test]
    fn wait_timeout_and_cancel() {
        let mut g = Gate::new(
            StartMode::Auto,
            Duration::from_secs(10),
            Duration::from_secs(5),
            F,
            0,
        );
        run(&mut g, 0, 10_000, obs(true, false, true));
        assert_eq!(g.end_reason(), Some(EndReason::WaitTimeout));

        let mut g = Gate::new(
            StartMode::Auto,
            Duration::from_secs(10),
            Duration::from_secs(600),
            F,
            0,
        );
        let t = run(&mut g, 0, 6_000, obs(true, true, true));
        g.tick(
            t + 100,
            &Observation {
                cancel: true,
                ..obs(true, true, true)
            },
        );
        assert_eq!(g.end_reason(), Some(EndReason::Cancelled));
        assert!(g.has_recorded());
    }

    #[test]
    fn hotkey_starts_and_stops() {
        let mut g = Gate::new(
            StartMode::Hotkey,
            Duration::from_secs(300),
            Duration::from_secs(600),
            F,
            0,
        );
        let t = run(&mut g, 0, 15_000, obs(true, true, true));
        assert_eq!(g.phase(), Phase::WaitingForHotkey);
        let press = Observation {
            hotkey: true,
            ..obs(true, true, true)
        };
        g.tick(t + 100, &press);
        assert_eq!(
            g.phase(),
            Phase::Recording,
            "без прогріву: момент обрав користувач"
        );
        let t = run(&mut g, t + 100, t + 8_100, obs(true, true, true));
        g.tick(t + 100, &press);
        assert_eq!(g.end_reason(), Some(EndReason::Hotkey));
        let total: i64 = g.segments().iter().map(|(a, b)| b - a).sum();
        assert_eq!(total, 8_100);
    }

    #[test]
    fn hotkey_mode_pauses_on_alt_tab_and_respects_cap() {
        let mut g = Gate::new(
            StartMode::Hotkey,
            Duration::from_secs(5),
            Duration::from_secs(600),
            F,
            0,
        );
        g.tick(
            100,
            &Observation {
                hotkey: true,
                ..obs(true, true, true)
            },
        );
        let t = run(&mut g, 100, 2_100, obs(true, true, true));
        let t = run(&mut g, t, t + 3_000, obs(true, false, true));
        assert_eq!(g.phase(), Phase::Paused);
        run(&mut g, t, t + 20_000, obs(true, true, true));
        assert_eq!(
            g.end_reason(),
            Some(EndReason::Completed),
            "тривалість — максимум і для клавіші"
        );
    }

    #[test]
    fn immediate_counts_from_start() {
        let mut g = Gate::new(
            StartMode::Immediate,
            Duration::from_secs(2),
            Duration::from_secs(600),
            F,
            0,
        );
        assert_eq!(g.phase(), Phase::Recording);
        run(&mut g, 0, 5_000, obs(true, false, false));
        assert_eq!(g.end_reason(), Some(EndReason::Completed));
        assert_eq!(g.segments(), &[(0, 2_000)]);
    }
}
