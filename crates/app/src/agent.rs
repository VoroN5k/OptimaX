//! Агент у треї (етап 4 `docs/SERVICE.md`): `OptimaX.exe --agent` — легкий процес без вікна й
//! без WebView, один на сеанс користувача. Запускається при вході в Windows **лише за згодою**:
//! значення `OptimaX Agent` у `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` ставить і
//! прибирає перемикач у «Налаштуваннях» (за замовчуванням вимкнено, CLAUDE.md 5.6).
//!
//! 4а — іконка з меню («Відкрити OptimaX», «Вийти з агента»). 4б — режим гри й чорна скринька:
//! якщо агент працює, їх веде він (рішення користувача 2026-11-16), без відкритого вікна. Стежить,
//! лише коли це ввімкнено («Режим гри» в меню чи в «Іграх» — `agent_link::AgentConfig`) і служба
//! OptimaX доступна (зміни — лише через неї). Стан — у `agent\status.json` для вікна й
//! повідомлення поверх гри.

use crate::agent_link::{self, AgentStatus};
use crate::changes::ChangeHub;
use crate::dto::{GameModeEvent, RollbackResult};
use crate::error::CmdResult;
use crate::link::{Link, WatchHandle};
use crate::session::gamemode::Watcher;
use crate::session::{blackbox, Host, HostRef, SessionCore};
use crate::state::lock;
use optimax_core::i18n::{Catalog, Text};
use optimax_core::registry::{RegHive, RegValue, RegistryApi, RegistryWriteApi};
use optimax_core::toast::{Toaster, Tone};
use optimax_core::tray::{MenuItem, Tray, TrayEvent};
use optimax_core::CoreError;
use optimax_frametime::watch::{WatchEvent, WatchOptions};
use optimax_metrics::CpuTempSource;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
const CMD_GAMEMODE: u32 = 3;

/// Такт керування: налаштування, пауза від вікна, служба.
const CONTROL_EVERY: Duration = Duration::from_millis(500);
/// Як часто перечитувати профілі й налаштування програми (датчики), поки гри немає.
const RELOAD_EVERY: Duration = Duration::from_secs(10);
/// Скільки показувати повідомлення поверх гри.
const TOAST_FOR: Duration = Duration::from_secs(3);

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

/// Що показує трей.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayState {
    service: bool,
    gamemode: bool,
    watching: bool,
    game: Option<String>,
}

fn tooltip(catalog: &Catalog, s: &TrayState) -> String {
    let text = if !s.service {
        Text::new("agent.tooltip.no_service")
    } else if let Some(game) = &s.game {
        Text::new("agent.tooltip.active").arg("game", game)
    } else if s.watching {
        Text::new("agent.tooltip.watching")
    } else {
        Text::new("agent.tooltip")
    };
    catalog.render(&text)
}

fn menu(catalog: &Catalog, s: &TrayState) -> Vec<MenuItem> {
    vec![
        MenuItem::Item {
            id: CMD_OPEN,
            label: catalog.render(&Text::new("agent.menu.open")),
            checked: false,
            enabled: true,
            default: true,
        },
        MenuItem::Item {
            id: CMD_GAMEMODE,
            label: catalog.render(&Text::new("agent.menu.gamemode")),
            checked: s.gamemode,
            enabled: s.service,
            default: false,
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

/// Сесію веде агент: зміни — лише через службу, стан — у файл для вікна, повідомлення — поверх
/// гри.
struct AgentHost {
    core: SessionCore,
    status: Mutex<AgentStatus>,
    toaster: Option<Toaster>,
    catalog: Catalog,
    /// Читати температуру CPU (налаштування програми «Розширені датчики»).
    cpu_temp: AtomicBool,
}

impl AgentHost {
    /// Змінює стан і записує його для вікна.
    fn publish(&self, f: impl FnOnce(&mut AgentStatus)) {
        let mut s = lock(&self.status);
        f(&mut s);
        s.seq += 1;
        if let Err(e) = agent_link::write_status(&s) {
            tracing::warn!("агент: стан не записано: {e}");
        }
    }

    fn toast(&self, text: &Text, tone: Tone) {
        if let Some(t) = &self.toaster {
            t.show(&self.catalog.render(text), tone, TOAST_FOR);
        }
    }

    fn status(&self) -> AgentStatus {
        lock(&self.status).clone()
    }
}

impl Host for AgentHost {
    fn core(&self) -> &SessionCore {
        &self.core
    }

    fn busy(&self) -> bool {
        agent_link::pause_active()
    }

    fn cpu_temp(&self) -> Option<CpuTempSource> {
        let link = self.core.link.clone();
        self.cpu_temp
            .load(Ordering::SeqCst)
            .then(|| CpuTempSource(Arc::new(move || link.cpu_temp())))
    }

    fn start_watch(&self, options: WatchOptions) -> CmdResult<(WatchHandle, Receiver<WatchEvent>)> {
        self.core.link.start_watch(None, options)
    }

    fn gamemode_event(&self, event: GameModeEvent) {
        match &event {
            GameModeEvent::Started { game, .. } => self.toast(
                &Text::new("agent.toast.started").arg("game", game),
                Tone::Ok,
            ),
            GameModeEvent::Ended { game, .. } => self.toast(
                &Text::new("agent.toast.ended").arg("game", game),
                Tone::Info,
            ),
            GameModeEvent::NeedsConfirmation { .. } => {
                self.toast(&Text::new("agent.toast.confirm"), Tone::Warn);
            }
            GameModeEvent::Watching { .. } | GameModeEvent::Stopped => {}
        }
        self.publish(|s| s.apply(&event));
    }

    fn blackbox_changed(&self) {
        let view = blackbox::view(&self.core);
        self.publish(|s| s.blackbox = view);
    }
}

/// Після збою (свого чи вікна): доводить незавершені зміни й повертає залишки режиму гри. Лише
/// якщо вікно не відкрите (інакше це робить воно) і служба вже доступна.
fn recover(host: &AgentHost) {
    use optimax_tweaks::Changes;
    let mut engine = lock(&host.core.changes);
    match engine.recover_pending() {
        Ok(n) if n > 0 => tracing::warn!("агент: оброблено {n} незавершених записів журналу"),
        Ok(_) => {}
        Err(e) => tracing::warn!("агент: відновлення журналу не вдалося: {e}"),
    }
    let r: RollbackResult = optimax_gamemode::restore_leftovers(&mut *engine).into();
    if !r.rolled_back.is_empty() || !r.failed.is_empty() {
        tracing::warn!(
            "агент: повернуто залишків режиму гри: {}, не вдалося: {}",
            r.rolled_back.len(),
            r.failed.len()
        );
    }
}

/// Цикл керування: налаштування й служба → стежник, пауза від вікна → пауза скриньки, стан → трей.
fn control(host: &Arc<AgentHost>, tray: &Tray, stop: &AtomicBool) {
    let host_ref: HostRef = host.clone();
    let mut watcher: Option<Watcher> = None;
    let mut pause: Option<blackbox::Pause> = None;
    let mut recovered = false;
    let mut next_reload = Instant::now();
    let mut shown: Option<TrayState> = None;
    while !stop.load(Ordering::SeqCst) {
        let cfg = agent_link::read_config();
        *lock(&host.core.allow_without_restore) = cfg.allow_without_restore;
        let service = host.core.link.service_usable();
        if host.status().service != service {
            host.publish(|s| s.service = service);
        }
        if service && !recovered {
            recovered = true;
            if !optimax_core::instance::is_taken(agent_link::UI_MUTEX) {
                recover(host);
            }
        }
        let reload = Instant::now() >= next_reload;
        if reload {
            next_reload = Instant::now() + RELOAD_EVERY;
            host.cpu_temp.store(
                crate::settings::Settings::load().pawnio_sensors,
                Ordering::SeqCst,
            );
        }
        let want = cfg.gamemode && service;
        let in_game = host.status().active.is_some();
        match watcher.take() {
            Some(w) if !want => {
                tracing::info!("агент: режим гри вимкнено");
                w.shutdown();
            }
            // Профілі змінились (у «Іграх») — перезапуск, але не посеред гри.
            Some(w) if reload && !in_game => match crate::profiles::load(&host.core.link) {
                Ok(p) if p != w.profiles() => {
                    w.shutdown();
                    if !p.is_empty() {
                        watcher = start_watcher(&host_ref, p);
                    }
                }
                _ => watcher = Some(w),
            },
            Some(w) => watcher = Some(w),
            None if want && reload => match crate::profiles::load(&host.core.link) {
                Ok(p) if !p.is_empty() => watcher = start_watcher(&host_ref, p),
                Ok(_) => {}
                Err(e) => tracing::warn!("агент: профілі не прочитано: {}", e.message()),
            },
            None => {}
        }
        // Вікно записує гру — скринька агента чекає (одна сесія PresentMon на систему).
        match (agent_link::pause_active(), pause.is_some()) {
            (true, false) => pause = Some(blackbox::Pause::new(&host_ref)),
            (false, true) => pause = None,
            _ => {}
        }
        let status = host.status();
        let now = TrayState {
            service,
            gamemode: cfg.gamemode,
            watching: status.watching,
            game: status.active.map(|a| a.game),
        };
        if shown.as_ref() != Some(&now) {
            tray.update(&tooltip(&host.catalog, &now), menu(&host.catalog, &now));
            shown = Some(now);
        }
        std::thread::sleep(CONTROL_EVERY);
    }
    drop(pause);
    if let Some(w) = watcher {
        // Вихід з агента посеред гри: усе повертається, як і при зупинці режиму гри.
        w.shutdown();
    }
}

fn start_watcher(host: &HostRef, profiles: Vec<optimax_gamemode::GameProfile>) -> Option<Watcher> {
    tracing::info!("агент: режим гри стежить за {} іграми", profiles.len());
    match Watcher::start(host.clone(), profiles) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::error!("агент: стежник не запущено: {e}");
            None
        }
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
    let link = Arc::new(Link::new());
    let changes = Arc::new(Mutex::new(ChangeHub::new(link.clone())));
    // Дозвіл без точки відновлення — лише до кінця цього запуску агента.
    if let Err(e) = agent_link::update_config(|c| c.allow_without_restore = false) {
        tracing::warn!("агент: налаштування не записано: {e}");
    }
    let start_seq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let toaster = Toaster::new()
        .map_err(|e| tracing::warn!("агент: повідомлення поверх гри недоступні: {e}"))
        .ok();
    let host = Arc::new(AgentHost {
        core: SessionCore::new(link, changes),
        status: Mutex::new(AgentStatus {
            seq: start_seq,
            ..Default::default()
        }),
        toaster,
        catalog,
        cpu_temp: AtomicBool::new(false),
    });
    host.publish(|_| {});
    let initial = TrayState {
        service: false,
        gamemode: false,
        watching: false,
        game: None,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let tray = Arc::new(Tray::new(
        &tooltip(&host.catalog, &initial),
        menu(&host.catalog, &initial),
        tx,
    )?);
    tracing::info!("агент OptimaX {} запущено", env!("CARGO_PKG_VERSION"));
    let stop = Arc::new(AtomicBool::new(false));
    let controller = {
        let (host, tray, stop) = (host.clone(), tray.clone(), stop.clone());
        std::thread::Builder::new()
            .name("optimax-agent-control".into())
            .spawn(move || control(&host, &tray, &stop))?
    };
    for event in rx {
        match event {
            TrayEvent::Activate | TrayEvent::Command(CMD_OPEN) => open_ui(),
            TrayEvent::Command(CMD_GAMEMODE) => {
                if let Err(e) = agent_link::update_config(|c| c.gamemode = !c.gamemode) {
                    tracing::warn!("агент: налаштування не записано: {e}");
                }
            }
            TrayEvent::Command(CMD_EXIT) => break,
            TrayEvent::Command(_) => {}
        }
    }
    stop.store(true, Ordering::SeqCst);
    let _ = controller.join();
    host.publish(|s| {
        s.apply(&GameModeEvent::Stopped);
        s.service = false;
    });
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
    fn menu_and_tooltips_follow_state_and_exist_in_the_catalog() {
        let catalog = Catalog::ukrainian().unwrap();
        for key in [
            "agent.menu.open",
            "agent.menu.exit",
            "agent.menu.gamemode",
            "agent.tooltip",
            "agent.tooltip.no_service",
            "agent.tooltip.watching",
            "agent.tooltip.active",
            "agent.toast.started",
            "agent.toast.ended",
            "agent.toast.confirm",
        ] {
            assert!(catalog.contains(key), "{key}");
        }
        let mut s = TrayState {
            service: false,
            gamemode: true,
            watching: false,
            game: None,
        };
        let m = menu(&catalog, &s);
        assert_eq!(m.len(), 4);
        // Без служби режим гри з трею не вмикається.
        assert!(matches!(
            &m[1],
            MenuItem::Item {
                id: CMD_GAMEMODE,
                enabled: false,
                checked: true,
                ..
            }
        ));
        assert_eq!(
            tooltip(&catalog, &s),
            catalog.render(&Text::new("agent.tooltip.no_service"))
        );
        s.service = true;
        s.watching = true;
        s.game = Some("re9.exe".into());
        assert!(tooltip(&catalog, &s).contains("re9.exe"));
    }
}
