//! Локалізація: логіка повертає ключі з аргументами, текст береться з каталогу.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

const UK_JSON: &str = include_str!("../locales/uk.json");

/// Локалізований текст: ключ каталогу + іменовані аргументи для `{name}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    /// Ключ у каталозі, напр. `audit.ram.below_rated.title`.
    pub key: String,
    /// Аргументи для підстановки.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, String>,
}

impl Text {
    /// Текст без аргументів.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            args: BTreeMap::new(),
        }
    }

    /// Додає аргумент.
    #[must_use]
    pub fn arg(mut self, name: impl Into<String>, value: impl ToString) -> Self {
        self.args.insert(name.into(), value.to_string());
        self
    }

    /// Значення, яке не потребує перекладу (назва пристрою, версія тощо).
    pub fn raw(value: impl ToString) -> Self {
        Self::new("value.raw").arg("v", value)
    }
}

/// Каталог перекладів однієї мови.
#[derive(Debug, Clone)]
pub struct Catalog {
    entries: HashMap<String, String>,
}

impl Catalog {
    /// Вбудований український каталог.
    pub fn ukrainian() -> Result<Self, CoreError> {
        Self::from_json(UK_JSON)
    }

    /// Каталог із плаского JSON-об'єкта `{ "ключ": "текст" }`.
    pub fn from_json(json: &str) -> Result<Self, CoreError> {
        Ok(Self {
            entries: serde_json::from_str(json)?,
        })
    }

    /// Чи є ключ у каталозі.
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Рендерить текст. Невідомий ключ повертається як є (без паніки), щоб пропуск було видно.
    pub fn render(&self, text: &Text) -> String {
        let Some(template) = self.entries.get(&text.key) else {
            return text.key.clone();
        };
        let mut out = String::with_capacity(template.len());
        let mut rest = template.as_str();
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('}') {
                Some(end) => {
                    let placeholder = &after[..end];
                    match self.resolve(placeholder, &text.args) {
                        Some(v) => out.push_str(v),
                        None => {
                            out.push('{');
                            out.push_str(placeholder);
                            out.push('}');
                        }
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// `{name}` → значення аргументу; `{name|одна|дві|багато}` → форма слова за числом у `name`.
    fn resolve<'a>(
        &self,
        placeholder: &'a str,
        args: &'a BTreeMap<String, String>,
    ) -> Option<&'a str> {
        let mut parts = placeholder.split('|');
        let value = args.get(parts.next()?)?;
        let forms: Vec<&str> = parts.collect();
        if forms.is_empty() {
            return Some(value);
        }
        let n: u64 = value.trim().trim_start_matches('-').parse().ok()?;
        forms.get(plural_index_uk(n)).or(forms.last()).copied()
    }
}

/// Індекс множинної форми для української: 0 — «1 день», 1 — «2 дні», 2 — «5 днів».
fn plural_index_uk(n: u64) -> usize {
    let (m10, m100) = (n % 10, n % 100);
    if m10 == 1 && m100 != 11 {
        0
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses() {
        let c = Catalog::ukrainian().unwrap();
        assert!(c.contains("impact.high"));
        assert!(c.contains("value.raw"));
    }

    #[test]
    fn render_substitutes_args() {
        let c = Catalog::from_json(r#"{"k": "RAM {cur} з {max} MT/s"}"#).unwrap();
        let t = Text::new("k").arg("cur", 2133).arg("max", 3600);
        assert_eq!(c.render(&t), "RAM 2133 з 3600 MT/s");
    }

    #[test]
    fn render_ukrainian_plurals() {
        let c = Catalog::from_json(r#"{"k": "{n} {n|день|дні|днів}"}"#).unwrap();
        let r = |n: u32| c.render(&Text::new("k").arg("n", n));
        assert_eq!(r(1), "1 день");
        assert_eq!(r(21), "21 день");
        assert_eq!(r(2), "2 дні");
        assert_eq!(r(133), "133 дні");
        assert_eq!(r(5), "5 днів");
        assert_eq!(r(11), "11 днів");
        assert_eq!(r(12), "12 днів");
        assert_eq!(r(111), "111 днів");
        // Не число — плейсхолдер лишається видимим.
        assert_eq!(
            c.render(&Text::new("k").arg("n", "x")),
            "x {n|день|дні|днів}"
        );
    }

    #[test]
    fn render_keeps_missing_parts_visible() {
        let c = Catalog::from_json(r#"{"k": "a {x} b {"}"#).unwrap();
        assert_eq!(c.render(&Text::new("k")), "a {x} b {");
        assert_eq!(c.render(&Text::new("missing.key")), "missing.key");
    }
}
