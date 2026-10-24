//! Шаблони шляхів правил: змінні середовища (`%LOCALAPPDATA%`) і `*` як цілий компонент
//! (будь-яке одне ім'я, напр. профіль браузера). Інших шаблонів немає — менше несподіванок.

use crate::path::{self, PathError};
use std::collections::HashMap;

/// Змінні середовища, з яких розгортаються шаблони.
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: HashMap<String, String>,
}

/// Змінні, які можуть використовувати правила й чорний список.
const KNOWN_VARS: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "LOCALAPPDATA",
    "APPDATA",
    "USERPROFILE",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "OneDrive",
    "PUBLIC",
];

impl Env {
    /// Змінні поточного процесу.
    pub fn from_process() -> Self {
        let mut env = Self::default();
        for name in KNOWN_VARS {
            if let Some(v) = std::env::var_os(name).and_then(|v| v.into_string().ok()) {
                env.set(name, &v);
            }
        }
        env
    }

    /// Типова система для тестів: `C:\Windows`, користувач `C:\Users\Test`.
    pub fn for_tests() -> Self {
        let mut env = Self::default();
        for (k, v) in [
            ("SystemRoot", r"C:\Windows"),
            ("SystemDrive", "C:"),
            ("LOCALAPPDATA", r"C:\Users\Test\AppData\Local"),
            ("APPDATA", r"C:\Users\Test\AppData\Roaming"),
            ("USERPROFILE", r"C:\Users\Test"),
            ("ProgramData", r"C:\ProgramData"),
            ("ProgramFiles", r"C:\Program Files"),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("ProgramW6432", r"C:\Program Files"),
            ("OneDrive", r"C:\Users\Test\OneDrive"),
            ("PUBLIC", r"C:\Users\Public"),
        ] {
            env.set(k, v);
        }
        env
    }

    /// Задає змінну (ім'я без урахування регістру).
    pub fn set(&mut self, name: &str, value: &str) {
        self.vars
            .insert(name.to_ascii_uppercase(), value.to_string());
    }

    /// Значення змінної.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .get(&name.to_ascii_uppercase())
            .map(String::as_str)
    }

    /// Підставляє `%NAME%`. Невідома змінна — `None` (правило на цій системі не діє).
    pub fn expand(&self, pattern: &str) -> Option<String> {
        let mut out = String::with_capacity(pattern.len());
        let mut rest = pattern;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after.find('%')?;
            out.push_str(self.get(&after[..end])?);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }
}

/// Розгорнутий шаблон: `C:\` + компоненти, де `None` — будь-яке одне ім'я.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    drive: String,
    segs: Vec<Option<String>>,
}

impl Pattern {
    /// Розгортає й нормалізує шаблон. `*` дозволено лише як цілий компонент.
    pub fn parse(pattern: &str, env: &Env) -> Result<Self, PathError> {
        let expanded = env.expand(pattern).ok_or(PathError::NotLocalAbsolute)?;
        // Нормалізуємо, підставивши на місце `*` безпечне ім'я-заглушку.
        const ANY: &str = "{any}";
        let marked: Vec<&str> = expanded
            .split(['\\', '/'])
            .map(|s| if s == "*" { ANY } else { s })
            .collect();
        let norm = path::normalize(&marked.join("\\"))?;
        let drive = norm.get(..3).unwrap_or_default().to_string();
        let segs = path::components(&norm)
            .map(|s| (s != ANY).then(|| s.to_string()))
            .collect();
        Ok(Self { drive, segs })
    }

    /// Корінь диска шаблону (`C:\`).
    pub fn drive(&self) -> &str {
        &self.drive
    }

    /// Чи є в шаблоні `*`.
    pub fn has_wildcard(&self) -> bool {
        self.segs.iter().any(Option::is_none)
    }

    /// Кількість компонентів.
    pub fn len(&self) -> usize {
        self.segs.len()
    }

    /// Чи порожній (корінь диска).
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// Компоненти (`None` — `*`).
    pub fn segs(&self) -> &[Option<String>] {
        &self.segs
    }

    /// Шлях лежить у межах шаблону (дорівнює йому або всередині). Повертає корінь, що збігся.
    pub fn match_root(&self, p: &str) -> Option<String> {
        if !path::eq_ci(p.get(..3)?, &self.drive) {
            return None;
        }
        let comps: Vec<&str> = path::components(p).collect();
        if comps.len() < self.segs.len() {
            return None;
        }
        let mut root = self.drive.clone();
        for (seg, comp) in self.segs.iter().zip(&comps) {
            if let Some(s) = seg {
                if !path::eq_ci(s, comp) {
                    return None;
                }
            }
            root = path::join(&root, comp);
        }
        Some(root)
    }

    /// Чи лежить шлях у межах шаблону.
    pub fn contains(&self, p: &str) -> bool {
        self.match_root(p).is_some()
    }

    /// Шлях точно дорівнює якомусь кореню шаблону.
    pub fn equals(&self, p: &str) -> bool {
        path::depth(p) == self.segs.len() && self.contains(p)
    }

    /// Чи можуть два шаблони перетинатися: один є префіксом іншого (з урахуванням `*`).
    pub fn overlaps(&self, other: &Pattern) -> bool {
        path::eq_ci(&self.drive, &other.drive)
            && self
                .segs
                .iter()
                .zip(&other.segs)
                .all(|(a, b)| match (a, b) {
                    (Some(a), Some(b)) => path::eq_ci(a, b),
                    _ => true,
                })
    }

    /// `self` охоплює `other` (other не коротший і збігається з self на префіксі).
    pub fn covers(&self, other: &Pattern) -> bool {
        other.segs.len() >= self.segs.len() && self.overlaps(other)
    }

    /// Шаблон як рядок (для журналу).
    pub fn display(&self) -> String {
        let mut s = self.drive.clone();
        for (i, seg) in self.segs.iter().enumerate() {
            if i > 0 {
                s.push('\\');
            }
            s.push_str(seg.as_deref().unwrap_or("*"));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_env_and_wildcards() {
        let env = Env::for_tests();
        let p = Pattern::parse(r"%LOCALAPPDATA%\Google\Chrome\User Data\*\Cache", &env).unwrap();
        assert!(p.has_wildcard());
        assert_eq!(
            p.match_root(
                r"C:\Users\Test\AppData\Local\Google\Chrome\User Data\Default\Cache\f_0001"
            ),
            Some(r"C:\Users\Test\AppData\Local\Google\Chrome\User Data\Default\Cache".to_string())
        );
        assert!(!p.contains(r"C:\Users\Test\AppData\Local\Google\Chrome\User Data\Default\History"));
        assert!(p.equals(r"c:\users\test\appdata\local\google\chrome\user data\Profile 1\cache"));
    }

    #[test]
    fn unknown_variable_disables_pattern() {
        let env = Env::for_tests();
        assert!(Pattern::parse(r"%NOPE%\x", &env).is_err());
        assert!(Pattern::parse(r"%LOCALAPPDATA%\..\x", &env).is_err());
        assert!(Pattern::parse(r"%LOCALAPPDATA%\x*", &env).is_err());
    }

    #[test]
    fn overlap_and_cover() {
        let env = Env::for_tests();
        let users = Pattern::parse(r"%SystemDrive%\Users\*", &env).unwrap();
        let docs = Pattern::parse(r"%SystemDrive%\Users\*\Documents", &env).unwrap();
        let tmp = Pattern::parse(r"%LOCALAPPDATA%\Temp", &env).unwrap();
        assert!(users.covers(&docs));
        assert!(!docs.covers(&users));
        assert!(users.covers(&tmp));
        assert!(!docs.overlaps(&tmp));
    }
}
