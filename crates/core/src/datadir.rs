//! Тека даних OptimaX: `%ProgramData%\OptimaX\<SID користувача>`.
//!
//! Програма працює з правами адміністратора, тож усе, що вона читає зі своєї теки даних (журнал
//! змін, карантин, налаштування, профілі), має бути недоступним на запис звичайним процесам.
//! Тому тека створюється з явним дескриптором безпеки: власник — Адміністратори, повний доступ —
//! SYSTEM і Адміністратори, сам користувач — лише читання (щоб відкрити звіт у Провіднику).
//!
//! Звичайний користувач може створити теку в `ProgramData` раніше за нас. Тека, власник якої не
//! Адміністратори й не SYSTEM (або яка є посиланням), вважається підкинутою: її вміст не
//! читається, вона перейменовується в `OptimaX.untrusted-<час>`, а замість неї створюється чиста.
//!
//! Спільні для всіх користувачів файли програми (вбудований PresentMon) — у
//! [`shared_dir`] (`%ProgramData%\OptimaX\bin`), без доступу для звичайних користувачів.
//!
//! Раніше дані лежали в `%LOCALAPPDATA%\OptimaX` ([`legacy_dir`]) — звідти їх переносить `app`.

use crate::CoreError;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(windows)]
mod windows;

/// Ім'я теки програми.
pub const APP_DIR: &str = "OptimaX";

/// Тека даних програми (створюється й перевіряється при першому виклику).
///
/// Значення кешується на весь час роботи процесу: обчислення створює теки й перевіряє права,
/// а результат — незмінний шлях, однаковий для всіх частин програми.
pub fn data_dir() -> Result<&'static Path, CoreError> {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    if let Some(d) = DIR.get() {
        return Ok(d);
    }
    let dir = imp::program_data()?;
    let dir = secure_dir_on(&dir)?;
    Ok(DIR.get_or_init(|| dir))
}

/// Захищена тека користувача на заданому корені (`C:\ProgramData`, `D:\` для карантину на
/// іншому диску): `<base>\OptimaX\<SID>`. Створює й перевіряє обидва рівні.
pub fn secure_dir_on(base: &Path) -> Result<PathBuf, CoreError> {
    imp::secure_dir_on(base)
}

/// Спільна для всіх користувачів підтека `%ProgramData%\OptimaX\<name>` (напр. `bin` для
/// вбудованого PresentMon): лише SYSTEM і Адміністратори, без читання для користувачів.
/// `name` — одне ім'я без роздільників.
pub fn shared_dir(name: &str) -> Result<PathBuf, CoreError> {
    if name.is_empty() || name.contains(['\\', '/', ':']) || name == "." || name == ".." {
        return Err(CoreError::InvalidData(format!(
            "недопустиме ім'я теки: {name}"
        )));
    }
    imp::shared_dir(&imp::program_data()?, name)
}

/// Колишня тека даних `%LOCALAPPDATA%\OptimaX` (для одноразового перенесення).
pub fn legacy_dir() -> Option<PathBuf> {
    imp::local_app_data().map(|d| d.join(APP_DIR))
}

/// Ім'я, під яке відсувається підкинута тека.
fn untrusted_name(name: &str, now_unix: u64) -> String {
    format!("{name}.untrusted-{now_unix}")
}

#[cfg(windows)]
use self::windows as imp;

#[cfg(not(windows))]
mod imp {
    use crate::CoreError;
    use std::path::{Path, PathBuf};

    pub(super) fn program_data() -> Result<PathBuf, CoreError> {
        Err(CoreError::Unsupported("тека даних OptimaX"))
    }

    pub(super) fn secure_dir_on(_base: &Path) -> Result<PathBuf, CoreError> {
        Err(CoreError::Unsupported("тека даних OptimaX"))
    }

    pub(super) fn shared_dir(_base: &Path, _name: &str) -> Result<PathBuf, CoreError> {
        Err(CoreError::Unsupported("тека даних OptimaX"))
    }

    pub(super) fn local_app_data() -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_name_keeps_original_visible() {
        assert_eq!(untrusted_name("OptimaX", 5), "OptimaX.untrusted-5");
    }
}
