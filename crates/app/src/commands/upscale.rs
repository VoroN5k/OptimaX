//! Апскейл у застосунку: окремий потік-контролер (гаряча клавіша Ctrl+Shift+F10, запуск з
//! відліком з інтерфейсу, повідомлення поверх гри) і подія `upscale` зі станом для UI.
//!
//! Налаштування для гри беруться з її профілю (якщо задані), інакше — загальні.

use crate::error::{CmdError, CmdResult};
use crate::state::{lock, AppState};
use optimax_core::toast::{Toaster, Tone, DEFAULT_DURATION};
use optimax_core::{Catalog, Text};
use optimax_gamemode::ProfileStore;
use optimax_upscale::{
    EndReason, HotkeyToggle, ToggleEvent, UpscaleSettings, UpscaleStats, WindowInfo,
};
use serde::Serialize;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// Найдовший відлік перед запуском з інтерфейсу.
const MAX_DELAY_S: u32 = 30;
/// Як часто контролер перевіряє клавішу й стан.
const TICK: Duration = Duration::from_millis(100);
/// Як часто надсилати в UI лічильники працюючого сеансу.
const STATS_EVERY: Duration = Duration::from_millis(500);

/// Працюючий сеанс (для UI).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UpscaleRunning {
    /// Заголовок вікна гри.
    pub title: String,
    /// Ім'я exe гри.
    pub exe: String,
    /// Множник масштабу.
    pub scale: f64,
    /// Налаштування сеансу.
    pub settings: UpscaleSettings,
    /// Налаштування взято з профілю гри.
    pub from_profile: bool,
    /// Лічильники й виміри.
    pub stats: UpscaleStats,
}

/// Стан апскейлу для інтерфейсу (подія `upscale` і команда `upscale_state`).
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UpscaleView {
    /// Ctrl+Shift+F10 зареєстровано.
    pub hotkey: bool,
    /// Чому гаряча клавіша недоступна (зайнята іншою програмою).
    pub hotkey_error: Option<Text>,
    /// Скільки секунд лишилось до запуску з інтерфейсу.
    pub countdown: Option<u32>,
    /// Працюючий сеанс.
    pub running: Option<UpscaleRunning>,
    /// Останнє повідомлення (зупинено / не запустився: причина).
    pub notice: Option<Text>,
}

enum Cmd {
    Start {
        delay: Duration,
    },
    Cancel,
    Stop,
    Settings {
        settings: UpscaleSettings,
        hotkey: bool,
    },
}

/// Контролер апскейлу (живе весь час роботи програми).
pub struct Controller {
    tx: Sender<Cmd>,
    view: Arc<Mutex<UpscaleView>>,
    handle: Option<JoinHandle<()>>,
}

impl Controller {
    /// Запускає потік контролера.
    pub fn spawn(app: AppHandle, settings: UpscaleSettings, hotkey: bool) -> CmdResult<Self> {
        let (tx, rx) = mpsc::channel();
        let view = Arc::new(Mutex::new(UpscaleView::default()));
        let handle = {
            let view = Arc::clone(&view);
            std::thread::Builder::new()
                .name("optimax-upscale-ctl".into())
                .spawn(move || run(&app, &rx, &view, settings, hotkey))
                .map_err(|e| CmdError::Failed(e.to_string()))?
        };
        Ok(Self {
            tx,
            view,
            handle: Some(handle),
        })
    }

    /// Поточний стан.
    pub fn view(&self) -> UpscaleView {
        lock(&self.view).clone()
    }

    fn send(&self, cmd: Cmd) -> CmdResult<()> {
        self.tx
            .send(cmd)
            .map_err(|_| CmdError::Failed("контролер апскейлу не працює".into()))
    }

    /// Нові налаштування (для наступного запуску) і стан гарячої клавіші.
    pub fn apply_settings(&self, settings: UpscaleSettings, hotkey: bool) {
        let _ = self.send(Cmd::Settings { settings, hotkey });
    }

    /// Зупиняє сеанс і потік (при виході з програми).
    pub fn shutdown(mut self) {
        let (tx, _) = mpsc::channel();
        // Закритий канал завершує цикл контролера; сеанс зупиняється в ньому ж.
        drop(std::mem::replace(&mut self.tx, tx));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Налаштування з профілю гри (за ім'ям exe), якщо задані.
fn profile_settings(window: &WindowInfo) -> Option<UpscaleSettings> {
    if window.exe.is_empty() {
        return None;
    }
    ProfileStore::default_location()?
        .load()
        .ok()?
        .into_iter()
        .find(|p| p.exe.eq_ignore_ascii_case(&window.exe))?
        .upscale
}

struct Ctl<'a> {
    app: &'a AppHandle,
    view: &'a Mutex<UpscaleView>,
    toggle: HotkeyToggle,
    settings: UpscaleSettings,
    toaster: Option<Toaster>,
    catalog: Option<Catalog>,
}

impl Ctl<'_> {
    fn emit(&self) {
        let _ = self.app.emit("upscale", lock(self.view).clone());
    }

    fn update(&self, f: impl FnOnce(&mut UpscaleView)) {
        f(&mut lock(self.view));
        self.emit();
    }

    fn render(&self, text: &Text) -> String {
        self.catalog
            .as_ref()
            .map_or_else(|| text.key.clone(), |c| c.render(text))
    }

    fn toast(&self, text: &Text, tone: Tone) {
        if let Some(t) = &self.toaster {
            t.show(&self.render(text), tone, DEFAULT_DURATION);
        }
    }

    fn set_hotkey(&mut self, enabled: bool) {
        let result = self.toggle.set_hotkey(enabled);
        let has = self.toggle.has_hotkey();
        self.update(|v| {
            v.hotkey = has;
            v.hotkey_error = result.err().map(|e| e.text());
        });
    }

    fn handle(&mut self, event: ToggleEvent) {
        match event {
            ToggleEvent::Started {
                window,
                scale,
                settings,
            } => {
                let text = Text::new("toast.upscale.on")
                    .arg("filter", self.render(&settings.filter.text()))
                    .arg("scale", format!("{scale:.2}"));
                tracing::info!(
                    "апскейл: {} ({}) ×{scale:.2}, фільтр {:?}",
                    window.title,
                    window.exe,
                    settings.filter
                );
                self.toast(&text, Tone::Info);
                let from_profile = settings != self.settings;
                self.update(|v| {
                    v.notice = None;
                    v.running = Some(UpscaleRunning {
                        title: window.title,
                        exe: window.exe,
                        scale,
                        settings,
                        from_profile,
                        stats: UpscaleStats::default(),
                    });
                });
            }
            ToggleEvent::Stopped(reason) => {
                let (text, tone) = match &reason {
                    EndReason::Stopped => (Text::new("toast.upscale.off"), Tone::Info),
                    EndReason::WindowClosed => (Text::new("toast.upscale.closed"), Tone::Warn),
                    EndReason::Failed(detail) => {
                        tracing::warn!("апскейл зупинився через збій: {detail}");
                        (
                            Text::new("toast.upscale.failed")
                                .arg("reason", self.render(&Text::new("upscale.error.thread"))),
                            Tone::Danger,
                        )
                    }
                };
                self.toast(&text, tone);
                self.update(|v| {
                    v.running = None;
                    v.notice = Some(text);
                });
            }
            ToggleEvent::Failed(e) => {
                tracing::warn!("апскейл не запустився: {e}");
                let text = Text::new("toast.upscale.failed").arg("reason", self.render(&e.text()));
                self.toast(&text, Tone::Danger);
                self.update(|v| v.notice = Some(text));
            }
        }
    }

    fn refresh_stats(&self) {
        if let Some(stats) = self.toggle.active().map(optimax_upscale::Upscaler::stats) {
            self.update(|v| {
                if let Some(r) = v.running.as_mut() {
                    r.stats = stats;
                }
            });
        }
    }
}

fn run(
    app: &AppHandle,
    rx: &Receiver<Cmd>,
    view: &Mutex<UpscaleView>,
    settings: UpscaleSettings,
    hotkey: bool,
) {
    let mut toggle = HotkeyToggle::without_hotkey(settings);
    toggle.set_resolver(Box::new(profile_settings));
    let mut ctl = Ctl {
        app,
        view,
        toggle,
        settings,
        toaster: Toaster::new().ok(),
        catalog: Catalog::ukrainian().ok(),
    };
    ctl.set_hotkey(hotkey);

    let mut countdown: Option<Instant> = None;
    let mut last_stats = Instant::now();
    loop {
        match rx.recv_timeout(TICK) {
            Ok(Cmd::Start { delay }) => {
                countdown = Some(Instant::now() + delay);
                ctl.update(|v| v.notice = None);
            }
            Ok(Cmd::Cancel) => {
                countdown = None;
                ctl.update(|v| v.countdown = None);
            }
            Ok(Cmd::Stop) => {
                countdown = None;
                ctl.update(|v| v.countdown = None);
                if let Some(e) = ctl.toggle.stop() {
                    ctl.handle(e);
                }
            }
            Ok(Cmd::Settings { settings, hotkey }) => {
                ctl.settings = settings;
                ctl.toggle.set_settings(settings);
                ctl.set_hotkey(hotkey);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if let Some(at) = countdown {
            let left = at.saturating_duration_since(Instant::now());
            if left.is_zero() {
                countdown = None;
                ctl.update(|v| v.countdown = None);
                let e = ctl.toggle.start_foreground();
                ctl.handle(e);
            } else {
                let secs = u32::try_from(left.as_secs()).unwrap_or(u32::MAX) + 1;
                if lock(view).countdown != Some(secs) {
                    ctl.update(|v| v.countdown = Some(secs));
                }
            }
        }

        if let Some(e) = ctl.toggle.poll() {
            ctl.handle(e);
        }
        if ctl.toggle.active().is_some() && last_stats.elapsed() >= STATS_EVERY {
            last_stats = Instant::now();
            ctl.refresh_stats();
        }
    }
    if let Some(e) = ctl.toggle.stop() {
        ctl.handle(e);
    }
}

fn controller<'a>(
    state: &'a State<'_, AppState>,
) -> CmdResult<std::sync::MutexGuard<'a, Option<Controller>>> {
    let guard = lock(&state.upscale);
    if guard.is_none() {
        return Err(CmdError::Failed("контролер апскейлу не запущено".into()));
    }
    Ok(guard)
}

/// Поточний стан апскейлу.
#[tauri::command]
pub fn upscale_state(state: State<'_, AppState>) -> UpscaleView {
    lock(&state.upscale)
        .as_ref()
        .map(Controller::view)
        .unwrap_or_default()
}

/// Запуск через `delay_s` секунд для вікна, що тоді буде на передньому плані (час перейти в гру).
#[tauri::command]
pub fn upscale_start(state: State<'_, AppState>, delay_s: u32) -> CmdResult<()> {
    let delay = Duration::from_secs(u64::from(delay_s.min(MAX_DELAY_S)));
    let guard = controller(&state)?;
    guard
        .as_ref()
        .map_or(Ok(()), |c| c.send(Cmd::Start { delay }))
}

/// Скасувати відлік.
#[tauri::command]
pub fn upscale_cancel(state: State<'_, AppState>) -> CmdResult<()> {
    let guard = controller(&state)?;
    guard.as_ref().map_or(Ok(()), |c| c.send(Cmd::Cancel))
}

/// Зупинити апскейл.
#[tauri::command]
pub fn upscale_stop(state: State<'_, AppState>) -> CmdResult<()> {
    let guard = controller(&state)?;
    guard.as_ref().map_or(Ok(()), |c| c.send(Cmd::Stop))
}
