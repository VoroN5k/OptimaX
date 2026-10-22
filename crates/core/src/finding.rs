//! Знахідки аудиту/діагностики: рівень впливу, впевненість, докази.

use crate::i18n::Text;
use serde::{Deserialize, Serialize};

/// Рівень впливу знахідки на продуктивність.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    /// Інформаційно: стан у нормі або вплив не доведено.
    Info,
    /// Помірний вплив.
    Moderate,
    /// Високий вплив: варто виправити насамперед.
    High,
}

impl Impact {
    /// Ключ локалізації назви рівня.
    pub fn text_key(self) -> &'static str {
        match self {
            Self::Info => "impact.info",
            Self::Moderate => "impact.moderate",
            Self::High => "impact.high",
        }
    }
}

/// Впевненість у знахідці, 0–100 %.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Confidence(u8);

impl Confidence {
    /// Створює значення, обрізаючи його до 100.
    pub fn new(percent: u8) -> Self {
        Self(percent.min(100))
    }

    /// Значення у відсотках.
    pub fn percent(self) -> u8 {
        self.0
    }
}

/// Область, до якої належить знахідка.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Оперативна пам'ять.
    Memory,
    /// Монітор.
    Display,
    /// Відеокарта (шина, BAR).
    Gpu,
    /// Драйвер.
    Driver,
    /// Прошивка (BIOS/UEFI).
    Firmware,
    /// Живлення.
    Power,
    /// Налаштування Windows.
    Os,
}

/// Один доказ: підпис і значення (обидва локалізовані).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// Що саме виміряно.
    pub label: Text,
    /// Виміряне значення.
    pub value: Text,
}

impl Evidence {
    /// Створює доказ.
    pub fn new(label: Text, value: Text) -> Self {
        Self { label, value }
    }
}

/// Знахідка аудиту або діагностики.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Стабільний ідентифікатор (напр. `ram.below_rated`), до нього прив'язуються виправлення.
    pub id: String,
    /// Область.
    pub category: Category,
    /// Рівень впливу.
    pub impact: Impact,
    /// Впевненість у тому, що знахідка правильна.
    pub confidence: Confidence,
    /// Короткий заголовок.
    pub title: Text,
    /// Пояснення.
    pub detail: Text,
    /// Порада, якщо є що робити.
    pub advice: Option<Text>,
    /// Докази (виміряні значення).
    pub evidence: Vec<Evidence>,
}
