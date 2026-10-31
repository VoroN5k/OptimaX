//! Оновлення програми (виняток із CLAUDE.md 5.6/5.7, погоджено користувачем 2026-10-30):
//! перевірка `latest.json` на GitHub Releases лише за кнопкою або за явно ввімкненим
//! налаштуванням; встановлення — лише за кнопкою і лише інсталятора, підпис якого збігся з
//! вшитим публічним ключем (перевіряє `tauri-plugin-updater`, вимкнути неможливо).
//!
//! Оновлюється тільки копія, встановлена інсталятором NSIS. Портативна копія (zip) лише
//! дізнається про нову версію і відкриває сторінку релізу.
//! Події: `update-progress` (`{ downloaded, total }`).

use crate::dto::{AvailableUpdate, UpdateProgress, UpdateStatus};
use crate::error::{CmdError, CmdResult};
use crate::state::REGISTRY;
use optimax_core::registry::{RegHive, RegValue, RegistryApi};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// Звідки дозволено завантажувати інсталятор (додатковий захист поверх підпису).
const DOWNLOAD_PREFIX: &str = "https://github.com/VoroN5k/OptimaX/releases/download/";
/// Сторінка релізів для портативної версії.
const RELEASES_PAGE: &str = "https://github.com/VoroN5k/OptimaX/releases/latest";
/// Ключ, який пише інсталятор NSIS (`UNINSTKEY` у шаблоні Tauri).
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\OptimaX";

fn failed(e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(e.to_string())
}

/// Чи цю копію встановив інсталятор: тека exe збігається з `InstallLocation`.
fn is_installed() -> bool {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().to_ascii_lowercase()))
    else {
        return false;
    };
    [RegHive::LocalMachine, RegHive::CurrentUser]
        .into_iter()
        .filter_map(|hive| {
            REGISTRY
                .read_value(hive, UNINSTALL_KEY, "InstallLocation")
                .ok()
                .flatten()
        })
        .any(|v| match v {
            RegValue::String(s) | RegValue::ExpandString(s) => {
                let loc = s
                    .trim()
                    .trim_matches('"')
                    .trim_end_matches('\\')
                    .to_ascii_lowercase();
                loc == dir.trim_end_matches('\\')
            }
            _ => false,
        })
}

async fn find(app: &AppHandle) -> CmdResult<Option<tauri_plugin_updater::Update>> {
    let update = app
        .updater()
        .map_err(failed)?
        .check()
        .await
        .map_err(failed)?;
    if let Some(u) = &update {
        if !u.download_url.as_str().starts_with(DOWNLOAD_PREFIX) {
            return Err(CmdError::Forbidden(format!(
                "оновлення з неочікуваної адреси: {}",
                u.download_url
            )));
        }
    }
    Ok(update)
}

/// Перевіряє, чи є новіша версія. Мережевий запит — лише до GitHub.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> CmdResult<UpdateStatus> {
    let available = find(&app).await?.map(|u| AvailableUpdate {
        version: u.version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.date().to_string()),
    });
    if let Some(a) = &available {
        tracing::info!("доступна нова версія OptimaX: {}", a.version);
    }
    Ok(UpdateStatus {
        current: env!("CARGO_PKG_VERSION").into(),
        available,
        installed: is_installed(),
    })
}

/// Завантажує й запускає інсталятор нової версії; програма закривається (так влаштоване
/// встановлення на Windows). `version` — та, яку користувач бачив і підтвердив.
#[tauri::command]
pub async fn update_install(app: AppHandle, version: String) -> CmdResult<()> {
    if !is_installed() {
        return Err(CmdError::Forbidden(
            "портативну копію не оновлюємо — завантажте нову версію зі сторінки релізу".into(),
        ));
    }
    let Some(update) = find(&app).await? else {
        return Err(CmdError::NotFound("оновлень немає".into()));
    };
    if update.version != version {
        return Err(CmdError::Failed(format!(
            "на GitHub уже версія {} — перевірте оновлення ще раз",
            update.version
        )));
    }
    tracing::info!("встановлення оновлення {version}: завантаження інсталятора");
    let mut downloaded: u64 = 0;
    let progress = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress.emit("update-progress", UpdateProgress { downloaded, total });
            },
            || tracing::info!("інсталятор завантажено, підпис перевірено — запуск"),
        )
        .await
        .map_err(failed)
}

/// Відкриває сторінку релізів у браузері (адреса фіксована, довільні посилання не відкриваємо).
#[tauri::command]
pub fn update_open_page() -> CmdResult<()> {
    let explorer = optimax_core::sysdirs::explorer()
        .ok_or_else(|| failed("не вдалося визначити теку Windows"))?;
    std::process::Command::new(explorer)
        .arg(RELEASES_PAGE)
        .spawn()
        .map(|_| ())
        .map_err(failed)
}
