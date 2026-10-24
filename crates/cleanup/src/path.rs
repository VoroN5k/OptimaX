//! Шляхи Windows як рядки: нормалізація, порівняння без урахування регістру, вкладеність.
//!
//! Логіка не спирається на `std::path`, щоб правила й безпеку можна було тестувати на будь-якій ОС.
//! Нормалізація **відмовляє**, а не «виправляє»: `..`, `.`, альтернативні потоки (`:`), імена з
//! крапкою чи пробілом у кінці, символи шаблонів, UNC і пристрої — помилка, а не спроба зрозуміти.

use std::fmt;

/// Чому шлях відхилено.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// Порожній або з нуль-символом.
    Empty,
    /// Не абсолютний шлях на локальному диску (`X:\…`).
    NotLocalAbsolute,
    /// Компонент `.` або `..`.
    Traversal,
    /// Компонент із недозволеними символами (`:` потоку, шаблони, крапка/пробіл у кінці).
    BadComponent(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "порожній шлях"),
            Self::NotLocalAbsolute => write!(f, "не абсолютний шлях на локальному диску"),
            Self::Traversal => write!(f, "шлях містить «.» або «..»"),
            Self::BadComponent(c) => write!(f, "недозволений компонент шляху «{c}»"),
        }
    }
}

impl std::error::Error for PathError {}

/// Нормалізує шлях до вигляду `X:\a\b` (літера диска велика, без завершального `\`).
/// Корінь диска — `X:\`.
pub fn normalize(input: &str) -> Result<String, PathError> {
    if input.is_empty() || input.contains('\0') {
        return Err(PathError::Empty);
    }
    let s = input.replace('/', "\\");
    // `\\?\C:\…` — довгий шлях на локальному диску; решта (`\\?\UNC\`, `\\.\`, `\\server`) — ні.
    let s = match s.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None if s.starts_with(r"\\") => return Err(PathError::NotLocalAbsolute),
        None => s,
    };
    let mut chars = s.chars();
    let drive = chars
        .next()
        .filter(char::is_ascii_alphabetic)
        .ok_or(PathError::NotLocalAbsolute)?;
    if chars.next() != Some(':') || chars.next() != Some('\\') {
        return Err(PathError::NotLocalAbsolute);
    }
    let rest: String = chars.collect();
    let mut out = format!("{}:\\", drive.to_ascii_uppercase());
    let mut first = true;
    for seg in rest.split('\\').filter(|s| !s.is_empty()) {
        check_component(seg)?;
        if !first {
            out.push('\\');
        }
        out.push_str(seg);
        first = false;
    }
    Ok(out)
}

fn check_component(seg: &str) -> Result<(), PathError> {
    if seg == "." || seg == ".." {
        return Err(PathError::Traversal);
    }
    let bad_char = seg
        .chars()
        .any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 32);
    // Windows мовчки відкидає крапки й пробіли в кінці імені — це інший шлях під тим самим ім'ям.
    if bad_char || seg.ends_with('.') || seg.ends_with(' ') {
        return Err(PathError::BadComponent(seg.to_string()));
    }
    Ok(())
}

/// Порівняння без урахування регістру (NTFS за замовчуванням нечутлива до регістру).
pub fn eq_ci(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Ключ для словників: нижній регістр нормалізованого шляху.
pub fn key(path: &str) -> String {
    path.to_lowercase()
}

/// `child` дорівнює `root` або лежить усередині нього. Обидва — нормалізовані.
pub fn is_within(child: &str, root: &str) -> bool {
    let (c, r) = (child.to_lowercase(), root.to_lowercase());
    if c == r {
        return true;
    }
    if r.ends_with('\\') {
        // Корінь диска `X:\`.
        return c.starts_with(&r);
    }
    c.len() > r.len() && c.starts_with(&r) && c.as_bytes().get(r.len()) == Some(&b'\\')
}

/// `child` лежить усередині `root` і не дорівнює йому.
pub fn is_strictly_within(child: &str, root: &str) -> bool {
    is_within(child, root) && !eq_ci(child, root)
}

/// Додає компонент до нормалізованого шляху.
pub fn join(base: &str, name: &str) -> String {
    if base.ends_with('\\') {
        format!("{base}{name}")
    } else {
        format!("{base}\\{name}")
    }
}

/// Батьківський каталог (для кореня диска — `None`).
pub fn parent(path: &str) -> Option<&str> {
    let idx = path.rfind('\\')?;
    if idx + 1 == path.len() {
        return None;
    }
    if idx == 2 {
        // `C:\a` → `C:\`
        return path.get(..3);
    }
    path.get(..idx)
}

/// Останній компонент.
pub fn file_name(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// Літера диска нормалізованого шляху (`C`).
pub fn drive_letter(path: &str) -> Option<char> {
    path.chars().next().filter(char::is_ascii_alphabetic)
}

/// Компоненти після `X:\`.
pub fn components(path: &str) -> impl Iterator<Item = &str> {
    path.get(3..)
        .unwrap_or("")
        .split('\\')
        .filter(|s| !s.is_empty())
}

/// Глибина (корінь диска — 0).
pub fn depth(path: &str) -> usize {
    components(path).count()
}

/// Чи це корінь диска (`X:\`).
pub fn is_drive_root(path: &str) -> bool {
    path.len() == 3 && path.ends_with(":\\")
}

/// Ім'я файлу збігається з простим шаблоном (`*` — будь-які символи), без урахування регістру.
pub fn glob_name(pattern: &str, name: &str) -> bool {
    fn rec(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => rec(&p[1..], n) || (!n.is_empty() && rec(p, &n[1..])),
            (Some(a), Some(b)) => a == b && rec(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    rec(&p, &n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_separators_and_case_of_drive() {
        assert_eq!(
            normalize(r"c:/Users//Me\\Temp\").unwrap(),
            r"C:\Users\Me\Temp"
        );
        assert_eq!(normalize(r"\\?\C:\Windows").unwrap(), r"C:\Windows");
        assert_eq!(normalize(r"d:\").unwrap(), r"D:\");
    }

    #[test]
    fn rejects_traversal_and_tricks() {
        assert_eq!(normalize(r"C:\Temp\..\Windows"), Err(PathError::Traversal));
        assert_eq!(normalize(r"C:\Temp\.\x"), Err(PathError::Traversal));
        assert_eq!(normalize(r"C:/Temp/../Windows"), Err(PathError::Traversal));
        assert!(matches!(
            normalize(r"C:\Temp\file.txt:stream"),
            Err(PathError::BadComponent(_))
        ));
        assert!(matches!(
            normalize(r"C:\Temp\name."),
            Err(PathError::BadComponent(_))
        ));
        assert!(matches!(
            normalize(r"C:\Temp\name "),
            Err(PathError::BadComponent(_))
        ));
        assert!(matches!(
            normalize(r"C:\Temp\*"),
            Err(PathError::BadComponent(_))
        ));
        assert_eq!(
            normalize(r"\\server\share\x"),
            Err(PathError::NotLocalAbsolute)
        );
        assert_eq!(
            normalize(r"\\?\UNC\server\share"),
            Err(PathError::NotLocalAbsolute)
        );
        assert_eq!(
            normalize(r"\\.\PhysicalDrive0"),
            Err(PathError::NotLocalAbsolute)
        );
        assert_eq!(normalize(r"Temp\x"), Err(PathError::NotLocalAbsolute));
        assert_eq!(normalize(r"C:Temp"), Err(PathError::NotLocalAbsolute));
        assert_eq!(normalize(""), Err(PathError::Empty));
    }

    #[test]
    fn within_respects_component_boundaries() {
        assert!(is_within(r"C:\Temp\a", r"C:\Temp"));
        assert!(is_within(r"c:\temp\A", r"C:\Temp"));
        assert!(is_within(r"C:\Temp", r"C:\TEMP"));
        assert!(!is_within(r"C:\Temporary", r"C:\Temp"));
        assert!(is_within(r"C:\x", r"C:\"));
        assert!(!is_within(r"D:\x", r"C:\"));
        assert!(!is_strictly_within(r"C:\Temp", r"C:\Temp"));
    }

    #[test]
    fn parent_and_name() {
        assert_eq!(parent(r"C:\a\b"), Some(r"C:\a"));
        assert_eq!(parent(r"C:\a"), Some(r"C:\"));
        assert_eq!(parent(r"C:\"), None);
        assert_eq!(file_name(r"C:\a\b.txt"), "b.txt");
        assert_eq!(depth(r"C:\"), 0);
        assert_eq!(depth(r"C:\a\b"), 2);
        assert_eq!(join(r"C:\", "a"), r"C:\a");
    }

    #[test]
    fn glob() {
        assert!(glob_name("*.dmp", "Mini0101.DMP"));
        assert!(!glob_name("*.dmp", "x.dmp.txt"));
        assert!(glob_name("webcache*", "webcache_4430"));
        assert!(glob_name("*", ""));
    }
}
