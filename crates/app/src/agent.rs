//! Агент у треї (етап 4а `docs/SERVICE.md`): `OptimaX.exe --agent` — легкий процес без вікна й
//! без WebView. Іконка в треї з меню («Відкрити OptimaX», «Вийти з агента»), один на сеанс
//! користувача. Запускається при вході в Windows **лише за згодою**: значення `OptimaX Agent` у
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` ставить і прибирає перемикач у
//! «Налаштуваннях» (за замовчуванням вимкнено, CLAUDE.md 5.6).
//!
//! Режим гри й чорна скринька в агенті — етап 4б.

use optimax_core::i18n::{Catalog, Text};
use optimax_core::registry::{RegHive, RegValue, RegistryApi, RegistryWriteApi};
use optimax_core::tray::{MenuItem, Tray, TrayEvent};
use optimax_core::CoreError;
use std::path::Path;

/// Аргумент запуску агента.
pub const ARG: &str = "--agent";
/// М'ютекс «агент уже працює» (простір імен сеансу).
pub const MUTEX: &str = r"Local\OptimaX-Agent";
/// Ключ автозапуску поточного користувача.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Ім'я значення автозапуску агента.
pub const RUN_VALUE: &str = "OptimaX Agent";

const CMD_OPEN: u32 = 1;
const CMD_EXIT: u32 = 2;

/// Команда автозапуску: exe в лапках і [`ARG`].
pub fn autostart_command(exe: &Path) -> String {
    format!("\"{}\" {ARG}", exe.display())
}

/// Чи ввімкнено автозапуск агента (є значення в `Run`).
pub fn autostart_enabled(reg: &dyn RegistryApi) -> bool {
    matches!(
        reg.read_value(RegHive::CurrentUser, RUN_KEY, RUN_VALUE),
        Ok(Some(_))
    )
}

/// Вмикає автозапуск агента з `exe` або вимикає (`None`).
pub fn set_autostart(reg: &dyn RegistryWriteApi, exe: Option<&Path>) -> Result<(), CoreError> {
    match exe {
        Some(exe) => reg.write_value(
            RegHive::CurrentUser,
            RUN_KEY,
            RUN_VALUE,
            &RegValue::String(autostart_command(exe)),
        ),
        None => reg.delete_value(RegHive::CurrentUser, RUN_KEY, RUN_VALUE),
    }
}

/// Чи працює агент у цьому сеансі.
pub fn is_running() -> bool {
    optimax_core::instance::is_taken(MUTEX)
}

fn menu(catalog: &Catalog) -> Vec<MenuItem> {
    vec![
        MenuItem::Item {
            id: CMD_OPEN,
            label: catalog.render(&Text::new("agent.menu.open")),
            checked: false,
            enabled: true,
            default: true,
        },
        MenuItem::Separator,
        MenuItem::item(CMD_EXIT, catalog.render(&Text::new("agent.menu.exit"))),
    ]
}

/// Відкриває вікно OptimaX (той самий exe без аргументів; права й службу він вирішує сам). Якщо
/// вікно вже відкрите, новий процес лише виводить його на передній план і виходить.
fn open_ui() {
    // Право вивести вікно наперед є в агента (щойно клік у треї) — передаємо його далі.
    optimax_core::instance::allow_foreground_any();
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(&exe).spawn() {
                tracing::warn!("не вдалося відкрити OptimaX: {e}");
            }
        }
        Err(e) => tracing::warn!("не вдалося знайти exe OptimaX: {e}"),
    }
}

fn init_log() {
    if let Some(file) = crate::applog::open_file(crate::applog::AGENT) {
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .try_init();
    }
}

/// Точка входу `OptimaX.exe --agent`. Другий екземпляр у тому самому сеансі одразу виходить.
pub fn run() -> anyhow::Result<()> {
    init_log();
    let Some(_guard) = optimax_core::instance::acquire(MUTEX)? else {
        tracing::info!("агент OptimaX уже працює в цьому сеансі");
        return Ok(());
    };
    let catalog = Catalog::ukrainian()?;
    let (tx, rx) = std::sync::mpsc::channel();
    let tray = Tray::new(
        &catalog.render(&Text::new("agent.tooltip")),
        menu(&catalog),
        tx,
    )?;
    tracing::info!("агент OptimaX {} запущено", env!("CARGO_PKG_VERSION"));
    for event in rx {
        match event {
            TrayEvent::Activate | TrayEvent::Command(CMD_OPEN) => open_ui(),
            TrayEvent::Command(CMD_EXIT) => break,
            TrayEvent::Command(_) => {}
        }
    }
    drop(tray);
    tracing::info!("агент OptimaX зупинено");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::registry::MemRegistry;

    #[test]
    fn autostart_is_written_and_removed() {
        let mut reg = MemRegistry::new();
        reg.create_key(RegHive::CurrentUser, RUN_KEY);
        assert!(!autostart_enabled(&reg));
        let exe = Path::new(r"C:\Program Files\OptimaX\OptimaX.exe");
        set_autostart(&reg, Some(exe)).unwrap();
        assert!(autostart_enabled(&reg));
        assert_eq!(
            reg.read_value(RegHive::CurrentUser, RUN_KEY, RUN_VALUE)
                .unwrap(),
            Some(RegValue::String(
                r#""C:\Program Files\OptimaX\OptimaX.exe" --agent"#.into()
            ))
        );
        set_autostart(&reg, None).unwrap();
        assert!(!autostart_enabled(&reg));
        // Повторне вимкнення — не помилка.
        set_autostart(&reg, None).unwrap();
    }

    #[test]
    fn menu_texts_exist_in_the_catalog() {
        let catalog = Catalog::ukrainian().unwrap();
        for key in ["agent.menu.open", "agent.menu.exit", "agent.tooltip"] {
            assert!(catalog.contains(key), "{key}");
        }
        assert_eq!(menu(&catalog).len(), 3);
    }
}
