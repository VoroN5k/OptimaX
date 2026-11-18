//! Мінімальний парсер текстового формату Valve KeyValues (VDF), яким Steam пише
//! `libraryfolders.vdf` і `appmanifest_*.acf`: рядки в лапках, вкладені `{ }`, коментарі `//`.

use super::LibraryError;

/// Найбільша глибина вкладення: у файлах Steam — 3–4 рівні; більше — пошкоджений файл.
const MAX_DEPTH: usize = 32;

/// Вузол дерева VDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// Рядкове значення.
    Str(String),
    /// Вкладений блок: пари «ключ — вузол» у порядку файлу.
    Map(Vec<(String, Node)>),
}

impl Node {
    /// Дочірній вузол за ключем (без урахування регістру); `None` для рядка чи відсутнього ключа.
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Node::Str(_) => None,
        }
    }

    /// Рядкове значення вузла.
    pub fn str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            Node::Map(_) => None,
        }
    }

    /// Пари блоку (порожньо для рядка).
    pub fn pairs(&self) -> &[(String, Node)] {
        match self {
            Node::Map(pairs) => pairs,
            Node::Str(_) => &[],
        }
    }
}

/// Лексема VDF.
#[derive(Debug, PartialEq, Eq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn err(reason: &str) -> LibraryError {
    LibraryError::parse("VDF", reason)
}

fn tokenize(text: &str) -> Result<Vec<Token>, LibraryError> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => out.push(Token::Open),
            '}' => out.push(Token::Close),
            '"' => {
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => return Err(err("рядок без закривної лапки")),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => return Err(err("рядок без закривної лапки")),
                        },
                        Some(other) => s.push(other),
                    }
                }
                out.push(Token::Str(s));
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            _ => return Err(err("неочікуваний символ поза лапками")),
        }
    }
    Ok(out)
}

/// Розбирає пари до `}` (якщо `nested`) або до кінця лексем.
fn pairs(
    tokens: &mut std::vec::IntoIter<Token>,
    depth: usize,
    nested: bool,
) -> Result<Vec<(String, Node)>, LibraryError> {
    if depth > MAX_DEPTH {
        return Err(err("надто глибоке вкладення"));
    }
    let mut out = Vec::new();
    loop {
        let key = match tokens.next() {
            None if nested => return Err(err("блок без закривної дужки")),
            None => return Ok(out),
            Some(Token::Close) if nested => return Ok(out),
            Some(Token::Close) => return Err(err("зайва закривна дужка")),
            Some(Token::Open) => return Err(err("блок без ключа")),
            Some(Token::Str(k)) => k,
        };
        let value = match tokens.next() {
            Some(Token::Str(v)) => Node::Str(v),
            Some(Token::Open) => Node::Map(pairs(tokens, depth + 1, true)?),
            Some(Token::Close) | None => return Err(err("ключ без значення")),
        };
        out.push((key, value));
    }
}

/// Розбирає текст VDF. Корінь — [`Node::Map`] з парами верхнього рівня.
pub fn parse(text: &str) -> Result<Node, LibraryError> {
    let mut tokens = tokenize(text)?.into_iter();
    Ok(Node::Map(pairs(&mut tokens, 0, false)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDERS: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"D:\\Steam"
		"label"		""
		"apps"
		{
			"480"		"1906055"
			"228980"		"846241848"
		}
	}
	"1"
	{
		"path"		"E:\\SteamLibrary"
	}
}
"#;

    #[test]
    fn parse_nested_maps() {
        let root = parse(FOLDERS).unwrap();
        let lf = root.get("libraryfolders").unwrap();
        let first = lf.get("0").unwrap();
        assert_eq!(first.get("label").unwrap().str(), Some(""));
        let apps = first.get("apps").unwrap();
        assert_eq!(apps.get("228980").unwrap().str(), Some("846241848"));
        assert_eq!(
            lf.get("1").unwrap().get("path").unwrap().str(),
            Some(r"E:\SteamLibrary")
        );
    }

    #[test]
    fn parse_unescapes_backslashes() {
        let root = parse(FOLDERS).unwrap();
        let p = root
            .get("libraryfolders")
            .unwrap()
            .get("0")
            .unwrap()
            .get("path");
        assert_eq!(p.unwrap().str(), Some(r"D:\Steam"));
    }

    #[test]
    fn parse_skips_line_comments() {
        let text = "// коментар\n\"a\"\n{\n  \"b\" \"c\" // ще коментар\n}\n";
        let root = parse(text).unwrap();
        assert_eq!(root.get("a").unwrap().get("b").unwrap().str(), Some("c"));
    }

    #[test]
    fn parse_unescapes_quotes() {
        let root = parse(r#""a" "x\"y""#).unwrap();
        assert_eq!(root.get("a").unwrap().str(), Some("x\"y"));
    }

    #[test]
    fn parse_rejects_unclosed_brace() {
        assert!(parse("\"a\"\n{\n \"b\" \"c\"\n").is_err());
    }

    #[test]
    fn parse_rejects_unterminated_string() {
        assert!(parse("\"a\" \"bcd").is_err());
    }

    #[test]
    fn parse_rejects_stray_closing_brace() {
        assert!(parse("\"a\" \"b\" }").is_err());
    }

    #[test]
    fn parse_rejects_too_deep_nesting() {
        let mut text = String::new();
        for _ in 0..40 {
            text.push_str("\"k\" { ");
        }
        for _ in 0..40 {
            text.push_str("} ");
        }
        assert!(parse(&text).is_err());
    }

    #[test]
    fn get_is_case_insensitive() {
        let root = parse("\"AppState\" { \"Name\" \"x\" }").unwrap();
        assert_eq!(
            root.get("appstate").unwrap().get("name").unwrap().str(),
            Some("x")
        );
    }

    #[test]
    fn str_of_map_is_none() {
        let root = parse("\"a\" { }").unwrap();
        assert_eq!(root.get("a").unwrap().str(), None);
    }
}
