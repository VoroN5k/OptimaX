//! Знахідки аудиту/діагностики: рівень впливу, впевненість, докази.

use crate::i18n::Text;
use serde::{Deserialize, Serialize};

/// Рівень впливу знахідки на продуктивність.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// Процесор як вузьке місце.
    Cpu,
    /// Відеопам'ять.
    Vram,
    /// Тепловий або силовий тротлінг.
    Throttling,
    /// Нерівні кадри, мікрофризи.
    Stutter,
    /// Фонові процеси.
    Background,
    /// Обмеження частоти кадрів (V-Sync, ліміт FPS).
    FrameCap,
    /// Загальний результат діагностики.
    Summary,
    /// Мережа: з'єднання, Wi-Fi, затримка й втрати.
    Network,
    /// Батарея ноутбука.
    Battery,
    /// Апаратні помилки з журналу подій (WHEA).
    HardwareErrors,
    /// Вильоти програм, сині екрани й раптові вимкнення.
    Crashes,
}

/// Один доказ: підпис і значення (обидва локалізовані).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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

/// Розділ програми, куди веде наступний крок знахідки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    /// Апскейл.
    Upscale,
}

/// Що людина може зробити після поради (кнопка під нею). Вирішує бекенд.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NextStep {
    /// Перевірити A/B у «Порівнянні»: назви двох варіантів.
    Compare {
        /// «Як зараз» і варіант після зміни.
        variants: [Text; 2],
    },
    /// Відкрити розділ програми.
    Open {
        /// Розділ.
        tool: Tool,
    },
}

/// Будівник [`Finding`]: ключі тексту `<prefix>.<id>.title`, `.detail`, `.advice`
/// з однаковими аргументами, щоб шаблони в каталозі могли використовувати будь-який з них.
#[derive(Debug, Clone)]
pub struct FindingBuilder {
    prefix: &'static str,
    id: &'static str,
    category: Category,
    impact: Impact,
    confidence: u8,
    args: Vec<(&'static str, String)>,
    has_advice: bool,
    evidence: Vec<Evidence>,
    next: Option<NextStep>,
}

impl FindingBuilder {
    /// Новий будівник.
    pub fn new(
        prefix: &'static str,
        id: &'static str,
        category: Category,
        impact: Impact,
        confidence: u8,
    ) -> Self {
        Self {
            prefix,
            id,
            category,
            impact,
            confidence,
            args: Vec::new(),
            has_advice: false,
            evidence: Vec::new(),
            next: None,
        }
    }

    /// Аргумент для всіх текстів знахідки.
    #[must_use]
    pub fn arg(mut self, name: &'static str, value: impl ToString) -> Self {
        self.args.push((name, value.to_string()));
        self
    }

    /// Додати текст поради (`.advice`).
    #[must_use]
    pub fn advice(mut self) -> Self {
        self.has_advice = true;
        self
    }

    /// Наступний крок під порадою.
    #[must_use]
    pub fn next(mut self, step: NextStep) -> Self {
        self.next = Some(step);
        self
    }

    /// Додати доказ.
    #[must_use]
    pub fn evidence(mut self, label: Text, value: Text) -> Self {
        self.evidence.push(Evidence::new(label, value));
        self
    }

    /// Додати кілька доказів.
    #[must_use]
    pub fn evidence_list(mut self, items: impl IntoIterator<Item = Evidence>) -> Self {
        self.evidence.extend(items);
        self
    }

    /// Змінити впевненість (напр. після врахування додаткових ознак).
    #[must_use]
    pub fn confidence(mut self, percent: u8) -> Self {
        self.confidence = percent;
        self
    }

    /// Готова знахідка.
    pub fn build(self) -> Finding {
        let text = |suffix: &str| {
            self.args.iter().fold(
                Text::new(format!("{}.{}.{suffix}", self.prefix, self.id)),
                |t, (k, v)| t.arg(*k, v),
            )
        };
        Finding {
            id: self.id.to_string(),
            category: self.category,
            impact: self.impact,
            confidence: Confidence::new(self.confidence),
            title: text("title"),
            detail: text("detail"),
            advice: self.has_advice.then(|| text("advice")),
            evidence: self.evidence,
            next: self.next,
        }
    }
}

/// Знахідка аудиту або діагностики.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// Що людина може зробити далі (кнопка під порадою).
    #[serde(default)]
    pub next: Option<NextStep>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finding_without_next_deserializes() {
        let f = FindingBuilder::new("diag", "x", Category::Gpu, Impact::Info, 50).build();
        let mut v = serde_json::to_value(&f).expect("json");
        v.as_object_mut().expect("obj").remove("next");
        let back: Finding = serde_json::from_value(v).expect("old json");
        assert_eq!(back.next, None);
    }

    #[test]
    fn builder_sets_next() {
        let f = FindingBuilder::new("diag", "x", Category::Gpu, Impact::Info, 50)
            .next(NextStep::Open {
                tool: Tool::Upscale,
            })
            .build();
        assert_eq!(
            f.next,
            Some(NextStep::Open {
                tool: Tool::Upscale
            })
        );
        let step = NextStep::Compare {
            variants: [Text::new("a"), Text::new("b")],
        };
        let v = serde_json::to_value(&step).expect("json");
        assert_eq!(v["kind"], "compare");
        assert_eq!(v["variants"][1]["key"], "b");
        let v = serde_json::to_value(NextStep::Open {
            tool: Tool::Upscale,
        })
        .expect("json");
        assert_eq!(v, serde_json::json!({"kind": "open", "tool": "upscale"}));
    }
}
