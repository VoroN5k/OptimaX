//! Увімкнення / вимкнення апскейлу гарячою клавішею `Ctrl+Shift+F10` для вікна на передньому плані.
//! Повідомлення поверх гри показує викликач за подіями [`ToggleEvent`] (тексти — з i18n-каталогу).

use crate::error::UpscaleError;
use crate::geometry::{dest_rect, scale_factor};
use crate::session::{EndReason, Upscaler};
use crate::settings::UpscaleSettings;
use crate::window::{foreground_window, WindowInfo};
use optimax_core::input::GlobalHotkey;

/// Що сталося після натискання або само собою.
#[derive(Debug)]
pub enum ToggleEvent {
    /// Апскейл запущено.
    Started {
        /// Вікно гри.
        window: WindowInfo,
        /// Очікуваний множник (за розміром вікна й монітора).
        scale: f64,
        /// З якими налаштуваннями (загальні або з профілю гри).
        settings: UpscaleSettings,
    },
    /// Апскейл зупинено (натисканням або сам: гру закрили, збій).
    Stopped(EndReason),
    /// Не вдалося запустити.
    Failed(UpscaleError),
}

/// Налаштування для конкретної гри (за вікном), напр. з профілю гри; `None` — загальні.
pub type SettingsResolver = Box<dyn Fn(&WindowInfo) -> Option<UpscaleSettings> + Send>;

/// Перемикач апскейлу: гаряча клавіша + поточний сеанс.
pub struct HotkeyToggle {
    /// `None` — без гарячої клавіші (вимкнена в налаштуваннях або зайнята): лише кнопка в UI.
    hotkey: Option<GlobalHotkey>,
    settings: UpscaleSettings,
    resolver: Option<SettingsResolver>,
    active: Option<Upscaler>,
}

impl HotkeyToggle {
    /// Реєструє `Ctrl+Shift+F10`. Помилка, якщо комбінацію зайняла інша програма.
    pub fn new(settings: UpscaleSettings) -> Result<Self, UpscaleError> {
        let hotkey =
            GlobalHotkey::ctrl_shift_f10().map_err(|e| UpscaleError::Hotkey(e.to_string()))?;
        Ok(Self {
            hotkey: Some(hotkey),
            settings,
            resolver: None,
            active: None,
        })
    }

    /// Перемикач без гарячої клавіші (керування лише з інтерфейсу).
    pub fn without_hotkey(settings: UpscaleSettings) -> Self {
        Self {
            hotkey: None,
            settings,
            resolver: None,
            active: None,
        }
    }

    /// Чи зареєстрована гаряча клавіша.
    pub fn has_hotkey(&self) -> bool {
        self.hotkey.is_some()
    }

    /// Вмикає / вимикає гарячу клавішу. Помилка — комбінацію зайняла інша програма
    /// (перемикач далі працює без неї).
    pub fn set_hotkey(&mut self, enabled: bool) -> Result<(), UpscaleError> {
        if enabled == self.hotkey.is_some() {
            return Ok(());
        }
        self.hotkey = if enabled {
            Some(GlobalHotkey::ctrl_shift_f10().map_err(|e| UpscaleError::Hotkey(e.to_string()))?)
        } else {
            None
        };
        Ok(())
    }

    /// Звідки брати налаштування для конкретної гри (перед кожним запуском).
    pub fn set_resolver(&mut self, resolver: SettingsResolver) {
        self.resolver = Some(resolver);
    }

    /// Налаштування для наступного запуску (поточний сеанс не змінюється).
    pub fn set_settings(&mut self, settings: UpscaleSettings) {
        self.settings = settings;
    }

    /// Поточний сеанс, якщо працює.
    pub fn active(&self) -> Option<&Upscaler> {
        self.active.as_ref()
    }

    /// Перевіряє натискання й стан сеансу. Викликати періодично (~10 разів на секунду).
    pub fn poll(&mut self) -> Option<ToggleEvent> {
        if let Some(reason) = self.active.as_ref().and_then(Upscaler::end_reason) {
            self.active = None;
            return Some(ToggleEvent::Stopped(reason));
        }
        if self.hotkey.as_ref().is_some_and(GlobalHotkey::pressed) {
            return Some(self.toggle());
        }
        None
    }

    /// Вмикає апскейл для вікна на передньому плані або вимикає поточний.
    pub fn toggle(&mut self) -> ToggleEvent {
        match self.active.take() {
            Some(upscaler) => ToggleEvent::Stopped(upscaler.stop()),
            None => self.start_foreground(),
        }
    }

    /// Вимикає поточний сеанс, якщо він є.
    pub fn stop(&mut self) -> Option<ToggleEvent> {
        self.active.take().map(|u| ToggleEvent::Stopped(u.stop()))
    }

    /// Вмикає апскейл для вікна на передньому плані (якщо вже працює — нічого не робить).
    pub fn start_foreground(&mut self) -> ToggleEvent {
        if self.active.is_some() {
            return ToggleEvent::Failed(UpscaleError::Thread("апскейл уже працює".into()));
        }
        let Some(window) = foreground_window() else {
            return ToggleEvent::Failed(UpscaleError::NoWindow);
        };
        let settings = self
            .resolver
            .as_ref()
            .and_then(|r| r(&window))
            .unwrap_or(self.settings);
        match Upscaler::start(window.id, settings) {
            Ok(upscaler) => {
                self.active = Some(upscaler);
                let dest = dest_rect(window.client, window.monitor.size(), settings.fit);
                ToggleEvent::Started {
                    scale: scale_factor(window.client, dest),
                    settings,
                    window,
                }
            }
            Err(e) => ToggleEvent::Failed(e),
        }
    }
}
