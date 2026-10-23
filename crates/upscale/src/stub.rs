//! Заглушка для не-Windows: апскейл недоступний, логіку можна тестувати.

use crate::error::UpscaleError;
use crate::session::Shared;
use crate::settings::UpscaleSettings;
use crate::window::{WindowId, WindowInfo};
use std::sync::mpsc::Sender;

const NAME: &str = "апскейл (лише Windows)";

pub(crate) struct Event;

impl Event {
    pub(crate) fn new() -> Result<Self, UpscaleError> {
        Err(UpscaleError::Unsupported(NAME))
    }
    pub(crate) fn set(&self) {}
}

pub(crate) fn run(
    _target: WindowId,
    _settings: UpscaleSettings,
    _shared: &Shared,
    _wake: &Event,
    ready: Sender<Result<(), UpscaleError>>,
) {
    let _ = ready.send(Err(UpscaleError::Unsupported(NAME)));
}

pub(crate) fn foreground_window() -> Option<WindowInfo> {
    None
}

pub(crate) fn window_info(_id: WindowId) -> Option<WindowInfo> {
    None
}

pub(crate) fn main_window_of(_pid: u32) -> Option<WindowInfo> {
    None
}

pub(crate) fn is_supported() -> bool {
    false
}

pub(crate) fn enable_per_monitor_dpi() {}
