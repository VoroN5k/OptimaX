//! Помилки апскейлу.

use crate::geometry::Size;
use optimax_core::Text;
use thiserror::Error;

/// Помилки модуля апскейлу.
#[derive(Debug, Error)]
pub enum UpscaleError {
    /// Система або платформа не вміє того, що потрібно (немає WGC, D3D 11.0 тощо).
    #[error("не підтримується: {0}")]
    Unsupported(&'static str),
    /// Вікна гри немає (закрили, або це не вікно верхнього рівня).
    #[error("вікно гри не знайдено")]
    NoWindow,
    /// Спроба масштабувати вікно самої OptimaX.
    #[error("не можна масштабувати вікно OptimaX")]
    OwnWindow,
    /// Вікно згорнуте — захоплювати нічого.
    #[error("вікно гри згорнуте")]
    Minimized,
    /// Вікно вже не менше за екран — масштабувати нікуди.
    #[error("вікно {client:?} вже не менше за екран {output:?}")]
    AlreadyFullSize {
        /// Розмір клієнтської області гри.
        client: Size,
        /// Розмір монітора.
        output: Size,
    },
    /// Виклик WinAPI / Direct3D / WinRT повернув помилку.
    #[error("{api} повернув помилку 0x{code:08X}")]
    Os {
        /// Назва функції.
        api: &'static str,
        /// HRESULT.
        code: u32,
    },
    /// Шейдер не скомпілювався (текст помилки від компілятора).
    #[error("помилка шейдера: {0}")]
    Shader(String),
    /// Гарячу клавішу `Ctrl+Shift+F10` не вдалося зареєструвати (зайнята іншою програмою).
    #[error("гаряча клавіша Ctrl+Shift+F10: {0}")]
    Hotkey(String),
    /// Робочий потік не стартував або завершився аварійно.
    #[error("потік апскейлу: {0}")]
    Thread(String),
}

impl UpscaleError {
    /// Пояснення для користувача (ключ i18n-каталогу `upscale.error.*`).
    pub fn text(&self) -> Text {
        match self {
            Self::Unsupported(what) => Text::new("upscale.error.unsupported").arg("what", what),
            Self::NoWindow => Text::new("upscale.error.no_window"),
            Self::OwnWindow => Text::new("upscale.error.own_window"),
            Self::Minimized => Text::new("upscale.error.minimized"),
            Self::AlreadyFullSize { client, .. } => Text::new("upscale.error.full_size")
                .arg("w", client.width)
                .arg("h", client.height),
            Self::Os { api, code } => Text::new("upscale.error.os")
                .arg("api", api)
                .arg("code", format!("{code:08X}")),
            Self::Shader(_) => Text::new("upscale.error.shader"),
            Self::Hotkey(_) => Text::new("upscale.error.hotkey"),
            Self::Thread(_) => Text::new("upscale.error.thread"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::Catalog;

    #[test]
    fn every_error_has_a_catalog_text() -> Result<(), optimax_core::CoreError> {
        let catalog = Catalog::ukrainian()?;
        let all = [
            UpscaleError::Unsupported("x"),
            UpscaleError::NoWindow,
            UpscaleError::OwnWindow,
            UpscaleError::Minimized,
            UpscaleError::AlreadyFullSize {
                client: Size::new(2560, 1440),
                output: Size::new(2560, 1440),
            },
            UpscaleError::Os { api: "X", code: 1 },
            UpscaleError::Shader(String::new()),
            UpscaleError::Hotkey(String::new()),
            UpscaleError::Thread(String::new()),
        ];
        for e in all {
            let text = e.text();
            assert!(catalog.contains(&text.key), "{}", text.key);
            assert!(!catalog.render(&text).contains('{'), "{}", text.key);
        }
        Ok(())
    }
}
