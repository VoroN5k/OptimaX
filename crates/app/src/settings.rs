//! Налаштування інтерфейсу (`settings.json` у захищеній теці даних).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Рівень ефектів інтерфейсу (`docs/DESIGN.md` розд. 8). Під час запису й аналізу нескінченні
/// анімації й фон вимкнені за будь-якого рівня — це контракт, а не налаштування.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Effects {
    /// Усі анімації, фон і розмиття плиток.
    Full,
    /// Лише одноразові переходи: без нескінченних анімацій, фону й розмиття.
    Calm,
    /// Без анімацій; живі метрики оновлюються раз на секунду.
    Off,
}

/// Фоновий шар (`docs/DESIGN.md` розд. 9, 11). Працює лише з «Ефектами: Повні» і поза вимірюваннями.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Background {
    /// Декоративна поверхня «Шовк», без даних, підписів і цифр. Старі значення `real` і
    /// `decorative` (фон із доріжками потоків, прибраний 2026-11-16) читаються як увімкнений.
    #[default]
    #[serde(alias = "real", alias = "decorative")]
    On,
    /// Без фону.
    Off,
}

/// Палітра акценту: змінює лише акцентні кольори, семантичні (увага, GPU, помилка) — ні.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    /// Шампань (за замовчуванням).
    #[default]
    Champagne,
    /// Аврора.
    Aurora,
    /// Смарагд.
    Emerald,
}

/// Налаштування.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Settings {
    /// Ефекти інтерфейсу; `None` — за системним `prefers-reduced-motion` (зменшений рух → вимкнені).
    #[serde(default)]
    pub effects: Option<Effects>,
    /// Палітра акценту.
    #[serde(default)]
    pub palette: Palette,
    /// Фоновий шар.
    #[serde(default)]
    pub background: Background,
    /// Як починати запис гри (діагностика й A/B).
    #[serde(default)]
    pub record_start: optimax_frametime::StartMode,
    /// Звукові сигнали на старті й зупинці запису.
    #[serde(default = "yes")]
    pub record_sound: bool,
    /// Загальні налаштування апскейлу (профіль гри може мати власні).
    #[serde(default)]
    pub upscale: optimax_upscale::UpscaleSettings,
    /// Гаряча клавіша Ctrl+Shift+F10 для апскейлу (працює, поки відкрита OptimaX).
    #[serde(default = "yes")]
    pub upscale_hotkey: bool,
    /// Очищення диска.
    #[serde(default)]
    pub cleanup: CleanupSettings,
    /// Перевіряти оновлення при запуску (мережевий запит до GitHub; за замовчуванням вимкнено).
    #[serde(default)]
    pub update_check_on_start: bool,
    /// Перевіряти новіший драйвер NVIDIA при запуску (запит до nvidia.com; за замовчуванням вимкнено).
    #[serde(default)]
    pub driver_check_on_start: bool,
    /// Користувач вимикає ПК утриманням кнопки живлення: раптові вимкнення без синього екрана
    /// в «Чому вилітає?» для нього звичайні (Windows не відрізняє їх від зникнення живлення).
    #[serde(default)]
    pub power_off_by_button: bool,
    /// Оверлей у грі (FPS, 1% low, CPU/GPU); за замовчуванням вимкнено.
    #[serde(default)]
    pub overlay: bool,
    /// Знайомство з програмою пройдено або пропущено (більше не показується саме).
    #[serde(default)]
    pub onboarded: bool,
    /// Читати температуру CPU й частоти шини пам'яті через драйвер PawnIO (якщо користувач сам
    /// його встановив). За замовчуванням вимкнено: вмикається лише явною згодою.
    #[serde(default)]
    pub pawnio_sensors: bool,
    /// Підказку «встанови службу, щоб не було UAC» на «Головній» закрито назавжди.
    #[serde(default)]
    pub service_hint_dismissed: bool,
}

/// Налаштування очищення.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CleanupSettings {
    /// Скільки днів файли 🟡 лежать у карантині.
    #[serde(default = "quarantine_days")]
    pub quarantine_days: u32,
    /// Диск для карантину (`D:\`, файли в `D:\OptimaX\<SID>\Quarantine`); `None` — тека даних.
    #[serde(default)]
    pub quarantine_drive: Option<String>,
    /// Автоматично видаляти карантин після терміну (при старті програми, із записом у журналі).
    #[serde(default)]
    pub auto_purge: bool,
}

fn quarantine_days() -> u32 {
    7
}

impl Default for CleanupSettings {
    fn default() -> Self {
        Self {
            quarantine_days: quarantine_days(),
            quarantine_drive: None,
            auto_purge: false,
        }
    }
}

/// Старі поля: економний режим чи вимкнена анімація → «Вимкнені», низька якість → «Спокійні».
/// Типові значення (`high`, без економії) нічого не задають: діє системне налаштування руху.
fn legacy_effects(o: &serde_json::Map<String, serde_json::Value>) -> Option<Effects> {
    let economy = o.get("economy").and_then(serde_json::Value::as_bool) == Some(true);
    match (
        economy,
        o.get("animation").and_then(serde_json::Value::as_str),
    ) {
        (true, _) | (_, Some("off")) => Some(Effects::Off),
        (_, Some("low")) => Some(Effects::Calm),
        _ => None,
    }
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            effects: None,
            palette: Palette::default(),
            background: Background::default(),
            record_start: optimax_frametime::StartMode::Auto,
            record_sound: true,
            upscale: optimax_upscale::UpscaleSettings::default(),
            upscale_hotkey: true,
            cleanup: CleanupSettings::default(),
            update_check_on_start: false,
            driver_check_on_start: false,
            power_off_by_button: false,
            overlay: false,
            onboarded: false,
            pawnio_sensors: false,
            service_hint_dismissed: false,
        }
    }
}

fn path() -> Option<PathBuf> {
    optimax_core::datadir::ui_dir()
        .ok()
        .map(|d| d.join("settings.json"))
}

impl Settings {
    /// Завантажує або повертає типові.
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    /// Розбирає `settings.json`, переносячи старі «Анімацію фону» й «Економний режим» в «Ефекти».
    fn parse(json: &str) -> Option<Self> {
        let mut v: serde_json::Value = serde_json::from_str(json).ok()?;
        if let Some(o) = v.as_object_mut() {
            if !o.contains_key("effects") {
                if let Some(e) = legacy_effects(o) {
                    o.insert("effects".into(), serde_json::json!(e));
                }
            }
            o.remove("animation");
            o.remove("economy");
        }
        serde_json::from_value(v).ok()
    }

    /// Рідше оновлення живих метрик (раніше — «Економний режим»).
    pub fn slow_metrics(&self) -> bool {
        self.effects == Some(Effects::Off)
    }

    /// Зберігає.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = path() else {
            return Ok(());
        };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(p, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_economy_or_off_becomes_effects_off() {
        let s = Settings::parse(r#"{"economy":true,"animation":"high"}"#).expect("s");
        assert_eq!(s.effects, Some(Effects::Off));
        assert!(s.slow_metrics());
        let s = Settings::parse(r#"{"animation":"off"}"#).expect("s");
        assert_eq!(s.effects, Some(Effects::Off));
    }

    #[test]
    fn legacy_low_becomes_calm_and_defaults_follow_system() {
        let s = Settings::parse(r#"{"animation":"low"}"#).expect("s");
        assert_eq!(s.effects, Some(Effects::Calm));
        assert!(!s.slow_metrics());
        let s = Settings::parse(r#"{"animation":"high","economy":false}"#).expect("s");
        assert_eq!(s.effects, None);
        assert_eq!(Settings::parse("{}").expect("s"), Settings::default());
    }

    #[test]
    fn legacy_background_modes_become_on() {
        for old in ["real", "decorative", "on"] {
            let s = Settings::parse(&format!(r#"{{"background":"{old}"}}"#)).expect("s");
            assert_eq!(s.background, Background::On);
        }
        let s = Settings::parse(r#"{"background":"off"}"#).expect("s");
        assert_eq!(s.background, Background::Off);
    }

    #[test]
    fn new_fields_win_over_legacy_and_roundtrip() {
        let s =
            Settings::parse(r#"{"effects":"full","palette":"aurora","economy":true}"#).expect("s");
        assert_eq!(s.effects, Some(Effects::Full));
        assert_eq!(s.palette, Palette::Aurora);
        let json = serde_json::to_string(&s).expect("j");
        assert!(!json.contains("economy") && !json.contains("animation"));
        assert_eq!(Settings::parse(&json).expect("s"), s);
    }
}
