//! Перевірений набір змін. Кожна має опис впливу на продуктивність, нагрів і безпеку
//! (тексти в каталозі `tweak.<id>.*`). Нічого небезпечного: Defender, VBS, UAC тощо тут немає.

use optimax_core::process::{PriorityClass, ProcessInfo};
use optimax_core::service::ServiceState;
use optimax_core::Text;
use serde::{Deserialize, Serialize};

/// Ідентифікатор твіку.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TweakId {
    /// Активна схема живлення.
    PowerScheme,
    /// Ігровий режим Windows.
    GameMode,
    /// Пріоритет процесу (режим гри OptimaX).
    ProcessPriority,
    /// Тимчасова зупинка служби (режим гри OptimaX).
    Service,
    /// Очищення standby-пам'яті (режим гри OptimaX).
    StandbyPurge,
}

impl TweakId {
    /// Усі твіки каталогу.
    pub const ALL: [TweakId; 5] = [
        TweakId::PowerScheme,
        TweakId::GameMode,
        TweakId::ProcessPriority,
        TweakId::Service,
        TweakId::StandbyPurge,
    ];

    /// Рядковий ідентифікатор (для журналу й ключів тексту).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PowerScheme => "power_scheme",
            Self::GameMode => "game_mode",
            Self::ProcessPriority => "process_priority",
            Self::Service => "service",
            Self::StandbyPurge => "standby_purge",
        }
    }

    /// Пошук за рядком.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Опис твіку.
    pub fn info(self) -> TweakInfo {
        let key = |suffix: &str| Text::new(format!("tweak.{}.{suffix}", self.as_str()));
        TweakInfo {
            id: self,
            title: key("title"),
            what: key("what"),
            performance: key("performance"),
            heat: key("heat"),
            safety: key("safety"),
            // Ігровий режим Windows вмикає для гри під час її запуску; щоб A/B був чесним,
            // гру треба перезапускати між прогонами. Схема живлення діє одразу.
            restart_game_between_runs: matches!(self, Self::GameMode),
            requires_reboot: false,
        }
    }
}

/// Опис твіку для показу «що зміниться».
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TweakInfo {
    /// Ідентифікатор.
    pub id: TweakId,
    /// Назва.
    pub title: Text,
    /// Що саме робить.
    pub what: Text,
    /// Вплив на продуктивність.
    pub performance: Text,
    /// Вплив на нагрів і споживання.
    pub heat: Text,
    /// Безпека й зворотність.
    pub safety: Text,
    /// Для чесного A/B гру треба перезапускати між прогонами.
    pub restart_game_between_runs: bool,
    /// Потрібне перезавантаження ПК.
    pub requires_reboot: bool,
}

/// Бажаний стан.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tweak", content = "value", rename_all = "snake_case")]
pub enum Desired {
    /// Зробити активною схему з цим GUID.
    PowerScheme(String),
    /// Увімкнути / вимкнути ігровий режим.
    GameMode(bool),
    /// Пріоритет процесу.
    ProcessPriority {
        /// Процес.
        process: ProcessInfo,
        /// Новий пріоритет.
        priority: PriorityClass,
    },
    /// Стан служби (лише зі списку дозволених).
    Service {
        /// Ім'я.
        name: String,
        /// Бажаний стан.
        state: ServiceState,
    },
}

impl Desired {
    /// До якого твіку належить.
    pub fn tweak(&self) -> TweakId {
        match self {
            Self::PowerScheme(_) => TweakId::PowerScheme,
            Self::GameMode(_) => TweakId::GameMode,
            Self::ProcessPriority { .. } => TweakId::ProcessPriority,
            Self::Service { .. } => TweakId::Service,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip() {
        for t in TweakId::ALL {
            assert_eq!(TweakId::parse(t.as_str()), Some(t));
            assert_eq!(t.info().title.key, format!("tweak.{}.title", t.as_str()));
        }
        assert!(TweakId::GameMode.info().restart_game_between_runs);
    }

    #[test]
    fn every_tweak_text_is_localized() {
        let c = optimax_core::Catalog::ukrainian().unwrap();
        for t in TweakId::ALL {
            let i = t.info();
            for text in [i.title, i.what, i.performance, i.heat, i.safety] {
                assert!(c.contains(&text.key), "немає {}", text.key);
            }
        }
        for key in [
            "tweak.value.absent",
            "tweak.preview.where",
            "tweak.preview.old",
            "tweak.preview.new",
            "tweak.preview.performance",
            "tweak.preview.heat",
            "tweak.preview.safety",
            "tweak.preview.noop",
            "tweak.restore.created",
            "tweak.restore.failed",
        ] {
            assert!(c.contains(key), "немає {key}");
        }
        assert!(!TweakId::PowerScheme.info().restart_game_between_runs);
    }
}
