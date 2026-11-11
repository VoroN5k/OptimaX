//! Встановлення, стан і видалення служби (потрібні права адміністратора).
//!
//! Служба реєструється **лише** з `%ProgramFiles%\OptimaX\OptimaX.exe` — у теці, куди пишуть
//! тільки адміністратори. Портативна версія спершу копіює себе туди й лишає позначку
//! [`COPY_MARKER`]; видалення служби прибирає таку копію, а встановлене інсталятором — не чіпає.

use crate::protocol::{SERVICE_DISPLAY, SERVICE_NAME};
use crate::{install_dir, same_file, service_args, service_exe, valid_sid, ServiceError};
use serde::Serialize;
use std::ffi::OsString;
use std::time::{Duration, Instant};
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

/// Позначка, що exe у теці встановлення скопіювала портативна версія (а не інсталятор).
pub const COPY_MARKER: &str = "service-copy.marker";
/// Скільки чекати на зупинку чи запуск служби.
const WAIT: Duration = Duration::from_secs(15);
/// Ключ, який пише інсталятор NSIS (там його тека встановлення).
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\OptimaX";

/// Тека, куди програму встановив інсталятор (`None` — інсталятора не було).
fn installer_dir() -> Option<std::path::PathBuf> {
    use optimax_core::registry::{RegHive, RegistryApi, WinRegistry};
    let v = WinRegistry
        .read_value(RegHive::LocalMachine, UNINSTALL_KEY, "InstallLocation")
        .ok()??;
    let s = v.as_str()?.trim().trim_matches('"').to_string();
    (!s.is_empty()).then(|| std::path::PathBuf::from(s))
}

/// Exe у теці встановлення належить інсталятору (а не є копією портативної версії).
fn installer_owns(dir: &std::path::Path) -> bool {
    installer_dir().is_some_and(|d| same_file(&d, dir))
}

fn os(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Os(e.to_string())
}

/// Стан служби в диспетчері служб.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SvcState {
    /// Не встановлена.
    NotInstalled,
    /// Зупинена.
    Stopped,
    /// Запускається чи зупиняється.
    Pending,
    /// Працює.
    Running,
}

/// Стан служби (без з'єднання з нею).
pub fn state() -> Result<SvcState, ServiceError> {
    let mgr =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(os)?;
    let Ok(svc) = mgr.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) else {
        return Ok(SvcState::NotInstalled);
    };
    let s = svc.query_status().map_err(os)?;
    Ok(match s.current_state {
        ServiceState::Running => SvcState::Running,
        ServiceState::Stopped => SvcState::Stopped,
        _ => SvcState::Pending,
    })
}

fn wait_for(svc: &windows_service::service::Service, want: ServiceState) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if svc.query_status().is_ok_and(|s| s.current_state == want) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Зупиняє службу, якщо вона є й працює (для оновлення exe).
pub fn stop() -> Result<(), ServiceError> {
    let mgr =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(os)?;
    let Ok(svc) = mgr.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP,
    ) else {
        return Ok(());
    };
    if svc
        .query_status()
        .is_ok_and(|s| s.current_state != ServiceState::Stopped)
    {
        let _ = svc.stop();
        if !wait_for(&svc, ServiceState::Stopped) {
            return Err(ServiceError::Os("служба не зупинилась вчасно".into()));
        }
    }
    Ok(())
}

/// Запускає встановлену службу (після оновлення exe). Заодно знімає відкладений автозапуск, який
/// ставили версії до 3д, — щоб оновлення інсталятором його прибрало без переустановлення.
pub fn start() -> Result<(), ServiceError> {
    let mgr =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(os)?;
    let Ok(svc) = mgr.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::CHANGE_CONFIG,
    ) else {
        return Ok(());
    };
    if let Err(e) = svc.set_delayed_auto_start(false) {
        tracing::warn!("служба: не вдалося зняти відкладений автозапуск: {e}");
    }
    if svc
        .query_status()
        .is_ok_and(|s| s.current_state == ServiceState::Stopped)
    {
        svc.start(&[] as &[&std::ffi::OsStr]).map_err(os)?;
        wait_for(&svc, ServiceState::Running);
    }
    Ok(())
}

/// Встановлює (або оновлює) службу для користувача `sid` і запускає її. Якщо програма запущена не
/// з теки встановлення — копіює себе туди. Повертає шлях exe служби.
pub fn install(sid: &str) -> Result<std::path::PathBuf, ServiceError> {
    if !valid_sid(sid) {
        return Err(ServiceError::Os(format!("некоректний SID: {sid}")));
    }
    let dir = install_dir().ok_or_else(|| os("не знайдено теку Program Files"))?;
    let target = service_exe().ok_or_else(|| os("не знайдено теку Program Files"))?;
    let me = std::env::current_exe().map_err(os)?;
    // Перед заміною exe служба має стояти.
    stop()?;
    if !same_file(&me, &target) {
        // Там уже програма з інсталятора: не перезаписувати її чужою копією (і не позначати як
        // «нашу копію», яку потім стерло б видалення служби). Копія без інсталятора (наша, що
        // лишилась після невдалого видалення) — перезаписується.
        if target.exists() && installer_owns(&dir) {
            return Err(ServiceError::Os(format!(
                "OptimaX уже встановлено інсталятором у {} — запустіть програму звідти й встановіть службу там",
                dir.display()
            )));
        }
        std::fs::create_dir_all(&dir).map_err(os)?;
        std::fs::copy(&me, &target).map_err(os)?;
        std::fs::write(dir.join(COPY_MARKER), b"").map_err(os)?;
        tracing::info!("служба: exe скопійовано в {}", target.display());
    }
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target.clone(),
        launch_arguments: service_args(sid).into_iter().map(OsString::from).collect(),
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let mgr = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(os)?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::QUERY_CONFIG;
    let svc = match mgr.open_service(SERVICE_NAME, access) {
        Ok(svc) => {
            svc.change_config(&info).map_err(os)?;
            svc
        }
        Err(_) => mgr.create_service(&info, access).map_err(os)?,
    };
    let _ = svc.set_description(
        "OptimaX: вимірювання для діагностики ігор. Встановлено за згодою користувача; видаляється в «Налаштуваннях» OptimaX.",
    );
    // Звичайний автозапуск (не відкладений): служба готова до входу в Windows, тож програма,
    // відкрита одразу після входу, не просить UAC. Поки з'єднання немає, служба лише чекає.
    let _ = svc.set_delayed_auto_start(false);
    svc.start(&[] as &[&std::ffi::OsStr]).map_err(os)?;
    if !wait_for(&svc, ServiceState::Running) {
        return Err(os("служба не запустилась (див. service.log)"));
    }
    tracing::info!("служба встановлена й запущена: {}", target.display());
    Ok(target)
}

/// Зупиняє й видаляє службу; копію exe, зроблену портативною версією, — теж.
pub fn uninstall() -> Result<(), ServiceError> {
    stop()?;
    let mgr =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(os)?;
    if let Ok(svc) = mgr.open_service(SERVICE_NAME, ServiceAccess::DELETE) {
        svc.delete().map_err(os)?;
    }
    if let (Some(dir), Some(exe)) = (install_dir(), service_exe()) {
        let marker = dir.join(COPY_MARKER);
        let me = std::env::current_exe().ok();
        // Не видаляти exe, з якого запущено цей процес, і програму з інсталятора.
        let running_copy = me.as_deref().is_some_and(|m| same_file(m, &exe));
        if marker.is_file() && !running_copy && !installer_owns(&dir) {
            // Процес служби міг ще не завершитися й тримати exe: кілька спроб. Позначку
            // прибираємо лише після exe — інакше копія лишиться «нічиєю».
            let deadline = Instant::now() + WAIT;
            while std::fs::remove_file(&exe).is_err() && exe.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(300));
            }
            if exe.exists() {
                tracing::warn!("копію {} не видалено (зайнята)", exe.display());
            } else {
                let _ = std::fs::remove_file(&marker);
                let _ = std::fs::remove_dir(&dir);
            }
        }
    }
    tracing::info!("служба видалена");
    Ok(())
}
