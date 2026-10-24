//! Що саме змінюється: місце, старе та нове значення.

use crate::catalog::TweakInfo;
use optimax_core::process::{PriorityClass, ProcessInfo};
use optimax_core::registry::{RegHive, RegValue};
use optimax_core::service::ServiceState;
use optimax_core::Text;
use serde::{Deserialize, Serialize};

/// Де зберігається налаштування.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SettingTarget {
    /// Значення реєстру.
    Registry {
        /// Корінь.
        hive: RegHive,
        /// Шлях ключа.
        path: String,
        /// Ім'я значення.
        name: String,
    },
    /// Активна схема живлення.
    PowerScheme,
    /// Пріоритет конкретного процесу (PID + час створення).
    ProcessPriority {
        /// Процес.
        process: ProcessInfo,
    },
    /// Стан служби Windows.
    Service {
        /// Ім'я служби.
        name: String,
    },
    /// Standby-список пам'яті (лише разова дія, без відкату).
    StandbyList,
    /// Оверлейний режим живлення (недокументований API powrprof.dll).
    PowerMode,
}

impl SettingTarget {
    /// Технічна адреса для показу й журналу (`HKCU\…\Value`).
    pub fn location(&self) -> String {
        match self {
            Self::Registry { hive, path, name } => {
                let root = match hive {
                    RegHive::LocalMachine => "HKLM",
                    RegHive::CurrentUser => "HKCU",
                };
                format!("{root}\\{path}\\{name}")
            }
            Self::PowerScheme => "powercfg: активна схема живлення".into(),
            Self::ProcessPriority { process } => {
                format!("процес {} (PID {}): пріоритет", process.name, process.pid)
            }
            Self::Service { name } => format!("служба {name}: стан"),
            Self::StandbyList => "пам'ять: standby-список".into(),
            Self::PowerMode => "powrprof: оверлейний режим живлення".into(),
        }
    }
}

/// Значення налаштування.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SettingValue {
    /// Значення реєстру; `None` — значення відсутнє (відкат = видалити).
    Registry(Option<RegValue>),
    /// GUID схеми живлення.
    PowerScheme(String),
    /// Клас пріоритету процесу.
    Priority(PriorityClass),
    /// Стан служби.
    Service(ServiceState),
    /// Разова дія без значення (очищення standby).
    Action,
    /// GUID оверлейного режиму; `None` = збалансований / скидання.
    PowerMode(Option<String>),
}

/// Одна запланована зміна.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlannedChange {
    /// Твік, до якого належить зміна.
    pub tweak_id: String,
    /// Де.
    pub target: SettingTarget,
    /// Поточне значення (буде відновлене при відкаті).
    pub old: SettingValue,
    /// Нове значення.
    pub new: SettingValue,
    /// Людський опис старого значення.
    pub old_display: Text,
    /// Людський опис нового значення.
    pub new_display: Text,
}

/// План: що зміниться і як це впливає. Показується користувачу **до** застосування.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Plan {
    /// Опис твіку (вплив на продуктивність, нагрів, безпеку).
    pub info: TweakInfo,
    /// Зміни. Порожньо — система вже в потрібному стані.
    pub changes: Vec<PlannedChange>,
}

impl Plan {
    /// Чи є що застосовувати.
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Опис значення реєстру: `REG_DWORD 0` або «значення відсутнє».
pub fn describe_registry(value: Option<&RegValue>) -> Text {
    match value {
        None => Text::new("tweak.value.absent"),
        Some(RegValue::Dword(v)) => Text::raw(format!("REG_DWORD {v} (0x{v:08X})")),
        Some(RegValue::Qword(v)) => Text::raw(format!("REG_QWORD {v}")),
        Some(RegValue::String(s)) => Text::raw(format!("REG_SZ \"{s}\"")),
        Some(other) => Text::raw(other.type_name()),
    }
}

/// Опис стану служби.
pub fn describe_service(state: ServiceState) -> Text {
    match state {
        ServiceState::Running => Text::new("service.running"),
        ServiceState::Stopped => Text::new("service.stopped"),
        ServiceState::Other(code) => Text::new("service.other").arg("code", code),
    }
}

/// Опис схеми живлення: назва й GUID.
pub fn describe_scheme(guid: &str, name: Option<&str>) -> Text {
    match name {
        Some(n) => Text::raw(format!("{n} ({guid})")),
        None => Text::raw(guid),
    }
}

/// Опис оверлейного режиму живлення.
pub fn describe_power_mode(guid: Option<&str>) -> Text {
    use optimax_core::power::overlay_guids as og;
    match guid {
        None => Text::new("power_mode.balanced"),
        Some(g) if g.eq_ignore_ascii_case(og::ECONOMY) => Text::new("power_mode.economy"),
        Some(g) if g.eq_ignore_ascii_case(og::BETTER_PERFORMANCE) => Text::new("power_mode.better"),
        Some(g) if g.eq_ignore_ascii_case(og::BEST_PERFORMANCE) => Text::new("power_mode.best"),
        Some(g) => Text::raw(g),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_and_descriptions() {
        let t = SettingTarget::Registry {
            hive: RegHive::CurrentUser,
            path: r"Software\Microsoft\GameBar".into(),
            name: "AutoGameModeEnabled".into(),
        };
        assert_eq!(
            t.location(),
            r"HKCU\Software\Microsoft\GameBar\AutoGameModeEnabled"
        );
        assert_eq!(describe_registry(None).key, "tweak.value.absent");
        assert_eq!(
            describe_registry(Some(&RegValue::Dword(1))).args["v"],
            "REG_DWORD 1 (0x00000001)"
        );
        assert_eq!(
            describe_scheme("g", Some("Balanced")).args["v"],
            "Balanced (g)"
        );
    }
}
