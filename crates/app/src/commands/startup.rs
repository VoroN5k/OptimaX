//! Екран «Автозавантаження»: що стартує з Windows і скільки воно зараз споживає.
//! Увімкнення / вимкнення — звичайний план твіку `startup_app` (журнал, відкат, точка
//! відновлення); тут лише перелік і перевірка, що запис справді існує.

use super::blocking;
use crate::dto::{StartupEntry, StartupLock, StartupRunning};
use crate::error::{CmdError, CmdResult};
use crate::state::REGISTRY;
use optimax_core::registry::{RegValue, RegistryApi};
use optimax_tweaks::startup::{
    exe_from_command, is_enabled, is_plain_file_name, is_system_path, StartupSource,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Тека «Автозавантаження» для джерела-теки.
fn folder(source: StartupSource) -> Option<PathBuf> {
    let (var, sub) = match source {
        StartupSource::UserFolder => ("APPDATA", r"Microsoft\Windows\Start Menu\Programs\Startup"),
        StartupSource::CommonFolder => (
            "ProgramData",
            r"Microsoft\Windows\Start Menu\Programs\StartUp",
        ),
        StartupSource::UserRun | StartupSource::MachineRun | StartupSource::MachineRun32 => {
            return None
        }
    };
    std::env::var_os(var).map(|base| PathBuf::from(base).join(sub))
}

/// Записи одного джерела: (ім'я, командний рядок або шлях до файлу).
fn entries(source: StartupSource) -> Vec<(String, String)> {
    if let Some(run) = source.run_key() {
        let names = REGISTRY
            .value_names(source.hive(), run)
            .unwrap_or_else(|e| {
                tracing::warn!("автозапуск: не вдалося прочитати {run}: {e}");
                Vec::new()
            });
        return names
            .into_iter()
            .filter_map(
                |name| match REGISTRY.read_value(source.hive(), run, &name) {
                    Ok(Some(RegValue::String(s) | RegValue::ExpandString(s))) => Some((name, s)),
                    _ => None,
                },
            )
            .collect();
    }
    let Some(dir) = folder(source) else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    read.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (!name.eq_ignore_ascii_case("desktop.ini"))
                .then(|| (name, e.path().to_string_lossy().into_owned()))
        })
        .collect()
}

/// Поточне навантаження процесів за повним шляхом exe (нижній регістр).
fn running_by_exe() -> HashMap<String, StartupRunning> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    let kind = ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_exe(UpdateKind::OnlyIfNotSet);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(300)));
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as f32;
    let mut out: HashMap<String, StartupRunning> = HashMap::new();
    for p in sys.processes().values() {
        let Some(exe) = p.exe() else { continue };
        let e = out
            .entry(exe.to_string_lossy().to_ascii_lowercase())
            .or_insert(StartupRunning {
                processes: 0,
                cpu_percent: 0.0,
                memory_mib: 0,
            });
        e.processes += 1;
        e.cpu_percent += p.cpu_usage() / cores;
        e.memory_mib += p.memory() / (1024 * 1024);
    }
    out
}

/// Усі записи автозапуску з поточним станом і навантаженням.
#[tauri::command]
pub async fn startup_list() -> CmdResult<Vec<StartupEntry>> {
    blocking(|| {
        let system_root = std::env::var("SystemRoot").unwrap_or_default();
        let running = running_by_exe();
        let mut list = Vec::new();
        for source in StartupSource::ALL {
            let approved = source.approved_key();
            for (name, command) in entries(source) {
                let exe = if source.run_key().is_some() {
                    exe_from_command(&command)
                } else {
                    command.clone()
                };
                let flag = REGISTRY.read_value(source.hive(), &approved, &name);
                let (enabled, bad_flag) = match flag.as_ref().map(|v| is_enabled(v.as_ref())) {
                    Ok(Ok(on)) => (on, false),
                    _ => (true, true),
                };
                let lock = if is_system_path(&exe, &system_root) {
                    Some(StartupLock::System)
                } else if bad_flag {
                    Some(StartupLock::UnknownFlag)
                } else {
                    None
                };
                // Ярлик (.lnk) не розгортаємо — його процес за шляхом не знайти.
                let is_exe = exe.to_ascii_lowercase().ends_with(".exe");
                list.push(StartupEntry {
                    source,
                    exe_exists: (is_exe && !exe.contains('%'))
                        .then(|| std::path::Path::new(&exe).exists()),
                    running: is_exe
                        .then(|| running.get(&exe.to_ascii_lowercase()).cloned())
                        .flatten(),
                    name,
                    command,
                    exe,
                    enabled,
                    lock,
                });
            }
        }
        list.sort_by(|a, b| {
            a.lock
                .is_some()
                .cmp(&b.lock.is_some())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(list)
    })
    .await
}

/// Перед планом: запис із теки має справді лежати в теці (для Run це перевіряє рушій).
pub fn validate(source: StartupSource, name: &str) -> CmdResult<()> {
    let Some(dir) = folder(source) else {
        return Ok(());
    };
    if !is_plain_file_name(name) || !dir.join(name).is_file() {
        return Err(CmdError::Forbidden(format!(
            "{name}: такого файлу в теці автозавантаження немає"
        )));
    }
    Ok(())
}
