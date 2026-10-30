//! Звіт для відгуку: один JSON-файл, який користувач сам надсилає розробнику.
//!
//! Мережі немає. Перед записом із усіх рядків прибираються ім'я користувача, шлях профілю
//! й ім'я комп'ютера ([`redact`]); великий журнал очищення скорочується до підсумків
//! ([`summarize_cleanup_journal`]), щоб у звіт не потрапили тисячі шляхів до файлів.

use serde_json::Value;

/// Скільки останніх сеансів очищення потрапляє у звіт.
pub const CLEANUP_SESSIONS: usize = 20;
/// Скільки «проблемних» рядків (пропущено, помилка) лишається в кожному сеансі.
pub const CLEANUP_PROBLEM_LINES: usize = 20;
/// Коротші за це імена не замінюємо: заміна «ab» зіпсувала б увесь текст звіту.
const MIN_SECRET_LEN: usize = 3;

/// Що замінити у звіті: значення → мітка. Порядок важливий: довші значення (шлях профілю)
/// мають іти раніше за коротші (ім'я користувача всередині цього шляху).
pub fn secrets_from_env() -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for (var, label) in [
        ("USERPROFILE", "%USERPROFILE%"),
        ("USERNAME", "<user>"),
        ("COMPUTERNAME", "<computer>"),
    ] {
        if let Ok(v) = std::env::var(var) {
            out.push((v, label));
        }
    }
    out
}

/// Замінює всі входження секретів (без урахування регістру ASCII, як у шляхах Windows)
/// у кожному рядку й ключі JSON.
pub fn redact(value: &mut Value, secrets: &[(String, &str)]) {
    match value {
        Value::String(s) => *s = redact_str(s, secrets),
        Value::Array(items) => items.iter_mut().for_each(|v| redact(v, secrets)),
        Value::Object(map) => {
            let old = std::mem::take(map);
            for (k, mut v) in old {
                redact(&mut v, secrets);
                map.insert(redact_str(&k, secrets), v);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn redact_str(s: &str, secrets: &[(String, &str)]) -> String {
    let mut out = s.to_string();
    for (secret, label) in secrets {
        if secret.chars().count() < MIN_SECRET_LEN {
            continue;
        }
        out = replace_ascii_ci(&out, secret, label);
    }
    out
}

/// Заміна без урахування регістру ASCII, лише цілими словами (ім'я `admin` не чіпає
/// `administrator`). `to_ascii_lowercase` не змінює довжину в байтах, тож індекси збігів
/// у нижньому регістрі збігаються з індексами в оригіналі.
fn replace_ascii_ci(haystack: &str, needle: &str, with: &str) -> String {
    let hay = haystack.to_ascii_lowercase();
    let pat = needle.to_ascii_lowercase();
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    for (i, _) in hay.match_indices(&pat) {
        let end = i + pat.len();
        if word(haystack[..i].chars().next_back()) || word(haystack[end..].chars().next()) {
            continue;
        }
        out.push_str(&haystack[last..i]);
        out.push_str(with);
        last = end;
    }
    out.push_str(&haystack[last..]);
    out
}

/// Останні [`CLEANUP_SESSIONS`] сеансів журналу очищення без переліку видалених файлів:
/// замість `lines` — їх кількість і до [`CLEANUP_PROBLEM_LINES`] рядків, де щось пішло
/// не так (не `deleted` / `quarantined` / `restored`).
pub fn summarize_cleanup_journal(journal: Value) -> Value {
    let Value::Array(sessions) = journal else {
        return Value::Null;
    };
    let skip = sessions.len().saturating_sub(CLEANUP_SESSIONS);
    let out = sessions
        .into_iter()
        .skip(skip)
        .map(|mut s| {
            if let Value::Object(map) = &mut s {
                if let Some(Value::Array(lines)) = map.remove("lines") {
                    let total = lines.len();
                    let problems: Vec<Value> = lines
                        .into_iter()
                        .filter(|l| {
                            let kind = l.pointer("/outcome/kind").and_then(Value::as_str);
                            !matches!(kind, Some("deleted" | "quarantined" | "restored"))
                        })
                        .take(CLEANUP_PROBLEM_LINES)
                        .collect();
                    map.insert("lines_total".into(), total.into());
                    map.insert("problem_lines".into(), Value::Array(problems));
                }
            }
            s
        })
        .collect();
    Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn secrets() -> Vec<(String, &'static str)> {
        vec![
            (r"C:\Users\volov".into(), "%USERPROFILE%"),
            ("volov".into(), "<user>"),
            ("DESKTOP-ABC".into(), "<computer>"),
        ]
    }

    #[test]
    fn redacts_profile_user_and_computer_case_insensitively() {
        let mut v = json!({
            "path": r"c:\users\VOLOV\AppData\Local\Temp\x.tmp",
            "msg": "user volov on desktop-abc",
            "list": [r"C:\Users\volov\Desktop", 5, null],
            r"C:\Users\volov\key": true,
        });
        redact(&mut v, &secrets());
        assert_eq!(
            v,
            json!({
                "path": r"%USERPROFILE%\AppData\Local\Temp\x.tmp",
                "msg": "user <user> on <computer>",
                "list": [r"%USERPROFILE%\Desktop", 5, null],
                r"%USERPROFILE%\key": true,
            })
        );
    }

    #[test]
    fn short_secrets_and_non_ascii_text_are_safe() {
        let mut v = json!("Абв ab — тест ab");
        redact(&mut v, &[("ab".into(), "<user>")]);
        assert_eq!(v, json!("Абв ab — тест ab"));
        let mut v = json!("admin is administrator");
        redact(&mut v, &[("Admin".into(), "<user>")]);
        assert_eq!(v, json!("<user> is administrator"));
        let mut v = json!("Олег: C:\\Users\\Олег\\x");
        redact(&mut v, &[("Олег".into(), "<user>")]);
        assert_eq!(v, json!("<user>: C:\\Users\\<user>\\x"));
    }

    #[test]
    fn cleanup_journal_keeps_counts_and_problems_only() {
        let line = |kind: &str| json!({"path": "C:\\x", "outcome": {"kind": kind}});
        let sessions: Vec<Value> = (0..25)
            .map(|i| {
                json!({
                    "id": i,
                    "freed_bytes": 10,
                    "lines": [line("deleted"), line("skipped"), line("quarantined"), line("failed")],
                })
            })
            .collect();
        let out = summarize_cleanup_journal(Value::Array(sessions));
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), CLEANUP_SESSIONS);
        assert_eq!(arr[0]["id"], json!(5));
        assert_eq!(arr[0]["lines_total"], json!(4));
        assert!(arr[0].get("lines").is_none());
        let kinds: Vec<&str> = arr[0]["problem_lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["outcome"]["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["skipped", "failed"]);
        assert_eq!(summarize_cleanup_journal(json!({"x": 1})), Value::Null);
    }
}
