//! Опис оновлення з `latest.json` → структура для показу в інтерфейсі.
//!
//! `latest.json` не підписаний, тому в інтерфейс іде не HTML, а лише розібраний текст:
//! розділи (`### Додано`), пункти списку (`- …`, вкладені — з відступом) і в них звичайний
//! текст, **жирний** і `код`. Усе інше лишається текстом як є. UI малює це компонентами, без
//! вставки HTML, тож розмітка з мережі не може виконатися.
//!
//! Розділ `### Коротко` — опис для гравця простою мовою; інтерфейс показує його першим, а
//! решту (технічні подробиці) — за кнопкою.

use serde::Serialize;

/// Різновид розділу (визначає мітку й колір).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    /// «Коротко» — опис простою мовою.
    Summary,
    /// «Безпека».
    Security,
    /// «Додано».
    Added,
    /// «Виправлено».
    Fixed,
    /// «Змінено».
    Changed,
    /// Будь-який інший заголовок.
    Other,
}

/// Різновид фрагмента тексту.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    /// Звичайний текст.
    Text,
    /// `**жирний**`.
    Bold,
    /// `` `код` ``.
    Code,
}

/// Фрагмент тексту.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NoteSpan {
    /// Різновид.
    pub kind: SpanKind,
    /// Текст (без розмітки).
    pub text: String,
}

/// Пункт списку.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NoteItem {
    /// 0 — верхній рівень, 1 — вкладений пункт.
    pub depth: u8,
    /// Текст пункту.
    pub spans: Vec<NoteSpan>,
}

/// Розділ опису.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NoteSection {
    /// Різновид.
    pub kind: NoteKind,
    /// Заголовок як у тексті (порожній — текст до першого заголовка).
    pub title: String,
    /// Пункти.
    pub items: Vec<NoteItem>,
}

fn kind_of(title: &str) -> NoteKind {
    match title.trim().to_lowercase().as_str() {
        "коротко" => NoteKind::Summary,
        "безпека" => NoteKind::Security,
        "додано" => NoteKind::Added,
        "виправлено" => NoteKind::Fixed,
        "змінено" => NoteKind::Changed,
        _ => NoteKind::Other,
    }
}

/// Розбирає `**жирний**` і `` `код` ``. Незакрита розмітка лишається текстом.
pub fn spans(line: &str) -> Vec<NoteSpan> {
    let mut out: Vec<NoteSpan> = Vec::new();
    let mut text = String::new();
    let mut rest = line;
    let flush = |text: &mut String, out: &mut Vec<NoteSpan>| {
        if !text.is_empty() {
            out.push(NoteSpan {
                kind: SpanKind::Text,
                text: std::mem::take(text),
            });
        }
    };
    while let Some(c) = rest.chars().next() {
        let (marker, kind) = if rest.starts_with("**") {
            ("**", SpanKind::Bold)
        } else if c == '`' {
            ("`", SpanKind::Code)
        } else {
            text.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        };
        let after = &rest[marker.len()..];
        match after.find(marker) {
            Some(end) if end > 0 => {
                flush(&mut text, &mut out);
                out.push(NoteSpan {
                    kind,
                    text: after[..end].to_string(),
                });
                rest = &after[end + marker.len()..];
            }
            _ => {
                text.push_str(marker);
                rest = after;
            }
        }
    }
    flush(&mut text, &mut out);
    out
}

/// Розбирає опис оновлення на розділи й пункти.
pub fn parse(notes: &str) -> Vec<NoteSection> {
    let mut sections: Vec<NoteSection> = Vec::new();
    // Поточний пункт збирається з кількох рядків (перенесення в CHANGELOG).
    let mut item: Option<(u8, String)> = None;

    fn push_item(sections: &mut Vec<NoteSection>, item: &mut Option<(u8, String)>) {
        if let Some((depth, text)) = item.take() {
            if sections.is_empty() {
                sections.push(NoteSection {
                    kind: NoteKind::Other,
                    title: String::new(),
                    items: Vec::new(),
                });
            }
            if let Some(s) = sections.last_mut() {
                s.items.push(NoteItem {
                    depth,
                    spans: spans(text.trim()),
                });
            }
        }
    }

    for line in notes.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(title) = trimmed.strip_prefix("### ") {
            push_item(&mut sections, &mut item);
            sections.push(NoteSection {
                kind: kind_of(title),
                title: title.trim().to_string(),
                items: Vec::new(),
            });
        } else if let Some(text) = trimmed.strip_prefix("- ") {
            push_item(&mut sections, &mut item);
            item = Some((u8::from(indent >= 2), text.to_string()));
        } else if trimmed.is_empty() {
            push_item(&mut sections, &mut item);
        } else if let Some((_, text)) = item.as_mut() {
            text.push(' ');
            text.push_str(trimmed);
        } else {
            item = Some((0, trimmed.to_string()));
        }
    }
    push_item(&mut sections, &mut item);
    sections.retain(|s| !s.items.is_empty());
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &[NoteSpan]) -> String {
        s.iter().map(|x| x.text.as_str()).collect()
    }

    #[test]
    fn sections_items_and_wrapped_lines() {
        let md = "### Коротко\n- Графік ривків.\n\n### Додано\n- **Радар** у «Чому лагає?»: графік\n  часу кадру.\n- Причини:\n  - **RAM**;\n  - `MsMpEng.exe`.\n";
        let s = parse(md);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].kind, NoteKind::Summary);
        assert_eq!(s[1].kind, NoteKind::Added);
        assert_eq!(s[1].items.len(), 4);
        // Перенесений рядок зливається з пунктом.
        assert_eq!(
            plain(&s[1].items[0].spans),
            "Радар у «Чому лагає?»: графік часу кадру."
        );
        assert_eq!(s[1].items[0].spans[0].kind, SpanKind::Bold);
        assert_eq!(s[1].items[2].depth, 1);
        assert_eq!(s[1].items[3].spans[0].kind, SpanKind::Code);
        assert_eq!(s[1].items[3].spans[0].text, "MsMpEng.exe");
    }

    #[test]
    fn markup_and_html_stay_plain_text() {
        // Незакрита розмітка й HTML — просто текст; UI покаже їх як є, без виконання.
        let s = spans("a **b `c <script>x</script>");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].kind, SpanKind::Text);
        assert_eq!(s[0].text, "a **b `c <script>x</script>");
        assert!(spans("****").iter().all(|x| x.kind == SpanKind::Text));
    }

    #[test]
    fn text_without_headings_and_unknown_titles() {
        let s = parse("Просто опис без розмітки.\n\n### Щось нове\n- пункт\n### Порожній\n");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].title, "");
        assert_eq!(plain(&s[0].items[0].spans), "Просто опис без розмітки.");
        assert_eq!(s[1].kind, NoteKind::Other);
        assert!(parse("").is_empty());
    }
}
