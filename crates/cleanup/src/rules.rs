//! База правил очищення: дані (`rules/*.toml`, вбудовані в бінарник), валідація й чиста функція
//! «шлях → вердикт».
//!
//! Правило без `source` або `verified_on`, а також правило, що порушує чорний список чи має
//! несумісні поля, **понижується до ⚪** (нічого не видаляє) і повертається як помилка валідації.

use crate::guard::{Guard, Zone};
use crate::pattern::{Env, Pattern};
use serde::{Deserialize, Serialize};

/// Вбудовані файли правил.
const EMBEDDED: &[(&str, &str)] = &[
    ("temp.toml", include_str!("../rules/temp.toml")),
    ("windows.toml", include_str!("../rules/windows.toml")),
    ("drivers.toml", include_str!("../rules/drivers.toml")),
    ("browsers.toml", include_str!("../rules/browsers.toml")),
    ("dev.toml", include_str!("../rules/dev.toml")),
    ("shaders.toml", include_str!("../rules/shaders.toml")),
    ("dumps.toml", include_str!("../rules/dumps.toml")),
];

/// Служби, які правило може тимчасово зупинити (стан відновлюється після операції).
pub const CLEANUP_SERVICES: &[&str] = &["wuauserv"];

/// Вердикт: чи можна видаляти.
///
/// Порядок варіантів — від м'якого до найсуворішого: при збігу кількох правил береться найсуворіший.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "CleanupVerdict"))]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 🟢 Відновлюється саме або не потрібне: видаляється після сухого прогону й підтвердження.
    Safe,
    /// 🟡 Видалити можна, але є ціна: лише через карантин.
    Consequences,
    /// ⚪ Програма не знає, що це: ніколи не видаляється.
    Unknown,
    /// 🔴 Системне: лише для інформації.
    Protected,
}

/// Як очищати.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Видалення файлів (🟢) або карантин (🟡).
    Delete,
    /// Штатне очищення компонентів Windows: `DISM /StartComponentCleanup`.
    Dism,
    /// Вимкнення гібернації: `powercfg /hibernate off`.
    Powercfg,
    /// Лише вручну штатними засобами Windows (підказка в `manual_hint`).
    ManualOnly,
}

/// Категорія для екрана.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "CleanupCategory"))]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Тимчасові файли.
    Temp,
    /// Оновлення Windows.
    WindowsUpdate,
    /// Драйвери (NVIDIA/AMD).
    Drivers,
    /// Кеші браузерів і лаунчерів.
    BrowserCaches,
    /// Кеші розробника.
    DevCaches,
    /// Шейдер-кеші.
    ShaderCaches,
    /// Дампи падінь.
    CrashDumps,
    /// Системне.
    System,
    /// Невідоме.
    Unknown,
}

/// Правило з файлу бази.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Унікальний ідентифікатор.
    pub id: String,
    /// Категорія.
    pub category: Category,
    /// Шаблони шляхів (каталог — очищується вміст; файл — сам файл).
    pub paths: Vec<String>,
    /// Що це.
    pub description: String,
    /// Вердикт.
    pub verdict: Verdict,
    /// Чи відновиться саме.
    pub regenerates: bool,
    /// Що буде після видалення.
    pub consequences: String,
    /// Спосіб очищення.
    pub method: Method,
    /// Джерело (документація чи опис перевірки).
    #[serde(default)]
    pub source: Option<String>,
    /// На чому перевірено (версія Windows / драйвера, дата).
    #[serde(default)]
    pub verified_on: Option<String>,
    /// Лише файли, старші за N днів (за найновішою з міток створення / зміни / доступу).
    #[serde(default)]
    pub min_age_days: Option<u32>,
    /// Залишити N найновіших вкладених об'єктів кореня (напр. останню збірку драйвера).
    #[serde(default)]
    pub keep_latest: Option<u32>,
    /// Лише файли з такими іменами (`*.dmp`).
    #[serde(default)]
    pub file_patterns: Vec<String>,
    /// Служби, які треба тимчасово зупинити (лише з [`CLEANUP_SERVICES`]).
    #[serde(default)]
    pub stop_services: Vec<String>,
    /// Як очистити штатними засобами (для `manual_only` / `dism` / `powercfg`).
    #[serde(default)]
    pub manual_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    rule: Vec<Rule>,
}

/// Проблема валідації правила.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleIssue {
    /// Файл бази.
    pub file: String,
    /// Правило (порожньо — помилка розбору файлу).
    pub rule_id: String,
    /// Що не так.
    pub problem: String,
}

/// Правило, підготовлене для цієї системи.
#[derive(Debug, Clone)]
pub struct LoadedRule {
    /// Дані правила.
    pub rule: Rule,
    /// Розгорнуті шаблони (ті, що не розгорнулись, — пропущені).
    pub patterns: Vec<Pattern>,
    /// Правило понижене до ⚪ через помилку валідації.
    pub demoted: bool,
}

impl LoadedRule {
    /// Вердикт з урахуванням пониження.
    pub fn verdict(&self) -> Verdict {
        if self.demoted {
            Verdict::Unknown
        } else {
            self.rule.verdict
        }
    }

    /// Спосіб з урахуванням пониження.
    pub fn method(&self) -> Method {
        if self.demoted {
            Method::ManualOnly
        } else {
            self.rule.method
        }
    }

    /// Чи можна вибрати для видалення / карантину.
    pub fn deletable(&self) -> bool {
        self.method() == Method::Delete
            && matches!(self.verdict(), Verdict::Safe | Verdict::Consequences)
    }
}

/// Результат класифікації шляху.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// Найсуворіший вердикт серед збігів.
    pub verdict: Verdict,
    /// Індекс правила, яке дало вердикт.
    pub rule: Option<usize>,
    /// Корінь цього правила, у межах якого лежить шлях.
    pub root: Option<String>,
    /// Зона чорного списку.
    pub zone: Option<Zone>,
}

/// Набір правил + чорний список для цієї системи.
#[derive(Debug, Clone)]
pub struct RuleSet {
    rules: Vec<LoadedRule>,
    guard: Guard,
    env: Env,
}

fn blank(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(|s| s.trim().is_empty())
}

impl RuleSet {
    /// Вбудована база для змінних середовища `env`. Друге значення — проблеми валідації
    /// (у тестах/CI їх має бути нуль).
    pub fn embedded(env: Env) -> (Self, Vec<RuleIssue>) {
        Self::from_sources(env, EMBEDDED)
    }

    /// База з наданих TOML-текстів `(ім'я файлу, вміст)`.
    pub fn from_sources(env: Env, files: &[(&str, &str)]) -> (Self, Vec<RuleIssue>) {
        let guard = Guard::new(&env);
        let mut issues = Vec::new();
        let mut rules: Vec<LoadedRule> = Vec::new();
        for (file, text) in files {
            let parsed: RuleFile = match toml::from_str(text) {
                Ok(p) => p,
                Err(e) => {
                    issues.push(RuleIssue {
                        file: (*file).to_string(),
                        rule_id: String::new(),
                        problem: format!("не розбирається: {e}"),
                    });
                    continue;
                }
            };
            for rule in parsed.rule {
                let mut problems = Vec::new();
                if rules.iter().any(|r| r.rule.id == rule.id) {
                    problems.push("повторюваний id".to_string());
                }
                let patterns = Self::patterns(&rule, &env, &guard, &mut problems);
                Self::check_fields(&rule, &mut problems);
                for problem in &problems {
                    issues.push(RuleIssue {
                        file: (*file).to_string(),
                        rule_id: rule.id.clone(),
                        problem: problem.clone(),
                    });
                }
                if !problems.is_empty() {
                    tracing::warn!(
                        "правило очищення «{}» понижено до ⚪: {problems:?}",
                        rule.id
                    );
                }
                rules.push(LoadedRule {
                    demoted: !problems.is_empty(),
                    rule,
                    patterns,
                });
            }
        }
        (Self { rules, guard, env }, issues)
    }

    fn patterns(rule: &Rule, env: &Env, guard: &Guard, problems: &mut Vec<String>) -> Vec<Pattern> {
        if rule.paths.is_empty() {
            problems.push("немає шляхів".into());
        }
        let mut out = Vec::new();
        for p in &rule.paths {
            if !p.starts_with('%') {
                problems.push(format!("шлях «{p}» має починатися зі змінної середовища"));
                continue;
            }
            match Pattern::parse(p, env) {
                Ok(pat) => {
                    if rule.verdict != Verdict::Protected {
                        if let Some(why) = guard.rejects_rule_root(&pat) {
                            problems.push(format!("шлях «{p}» {why}"));
                        }
                    }
                    out.push(pat);
                }
                // Змінної немає на цій системі (напр. OneDrive) — правило тут просто не діє.
                Err(crate::path::PathError::NotLocalAbsolute) => {}
                Err(e) => problems.push(format!("шлях «{p}»: {e}")),
            }
        }
        out
    }

    fn check_fields(rule: &Rule, problems: &mut Vec<String>) {
        if blank(&rule.source) {
            problems.push("немає source".into());
        }
        if blank(&rule.verified_on) {
            problems.push("немає verified_on".into());
        }
        if rule.id.trim().is_empty() || rule.description.trim().is_empty() {
            problems.push("порожній id або description".into());
        }
        if rule.consequences.trim().is_empty() {
            problems.push("порожнє consequences".into());
        }
        let ok_method = match rule.method {
            Method::Delete => matches!(rule.verdict, Verdict::Safe | Verdict::Consequences),
            Method::Dism => rule.verdict == Verdict::Protected,
            Method::Powercfg => rule.verdict == Verdict::Consequences,
            Method::ManualOnly => true,
        };
        if !ok_method {
            problems.push(format!(
                "спосіб {:?} несумісний з вердиктом {:?}",
                rule.method, rule.verdict
            ));
        }
        if rule.method != Method::Delete
            && (rule.min_age_days.is_some()
                || rule.keep_latest.is_some()
                || !rule.file_patterns.is_empty()
                || !rule.stop_services.is_empty())
        {
            problems.push("умови видалення без method = delete".into());
        }
        if rule.method != Method::Delete && blank(&rule.manual_hint) {
            problems.push("немає manual_hint для штатного способу".into());
        }
        for s in &rule.stop_services {
            if !CLEANUP_SERVICES.iter().any(|a| a.eq_ignore_ascii_case(s)) {
                problems.push(format!("служба «{s}» не в дозволеному списку"));
            }
        }
    }

    /// Усі правила.
    pub fn rules(&self) -> &[LoadedRule] {
        &self.rules
    }

    /// Правило за індексом.
    pub fn get(&self, idx: usize) -> Option<&LoadedRule> {
        self.rules.get(idx)
    }

    /// Індекс правила за id.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.rules.iter().position(|r| r.rule.id == id)
    }

    /// Найбільша глибина шаблону: глибший шлях не може бути коренем жодного правила.
    pub fn max_pattern_depth(&self) -> usize {
        self.rules
            .iter()
            .flat_map(|r| r.patterns.iter().map(Pattern::len))
            .max()
            .unwrap_or(0)
    }

    /// Чорний список.
    pub fn guard(&self) -> &Guard {
        &self.guard
    }

    /// Змінні середовища.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Шлях → вердикт. Чиста функція: файлову систему не чіпає.
    ///
    /// Збіг із кількома правилами — найсуворіший вердикт. Без збігу — ⚪, а в забороненій зоні
    /// чорного списку — 🔴 (навіть якщо якесь правило вважає інакше).
    pub fn classify(&self, p: &str) -> Classification {
        let zone = self.guard.forbidden_zone(p);
        let mut best: Option<(Verdict, usize, String)> = None;
        for (i, r) in self.rules.iter().enumerate() {
            for pat in &r.patterns {
                if let Some(root) = pat.match_root(p) {
                    let v = r.verdict();
                    if best.as_ref().is_none_or(|(bv, _, _)| v > *bv) {
                        best = Some((v, i, root));
                    }
                }
            }
        }
        let (mut verdict, rule, root) = match best {
            Some((v, i, root)) => (v, Some(i), Some(root)),
            None => (Verdict::Unknown, None, None),
        };
        if zone.is_some() {
            verdict = Verdict::Protected;
        }
        Classification {
            verdict,
            rule,
            root,
            zone,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
[[rule]]
id = "tmp"
category = "temp"
paths = ['%LOCALAPPDATA%\Temp']
description = "Тимчасові файли"
verdict = "safe"
regenerates = true
consequences = "Нічого"
method = "delete"
source = "doc"
verified_on = "Windows 11 22631, 2026-10-24"

[[rule]]
id = "shader"
category = "shader_caches"
paths = ['%LOCALAPPDATA%\Temp\Shaders']
description = "Шейдери"
verdict = "consequences"
regenerates = true
consequences = "Підвисання"
method = "delete"
source = "doc"
verified_on = "Windows 11"
"#;

    fn set(text: &str) -> (RuleSet, Vec<RuleIssue>) {
        RuleSet::from_sources(Env::for_tests(), &[("t.toml", text)])
    }

    #[test]
    fn embedded_rules_are_valid() {
        let (rs, issues) = RuleSet::embedded(Env::for_tests());
        assert!(issues.is_empty(), "{issues:#?}");
        assert!(rs.rules().len() >= 10);
        assert!(rs.rules().iter().all(|r| !r.demoted));
    }

    #[test]
    fn classify_strictest_wins() {
        let (rs, issues) = set(GOOD);
        assert!(issues.is_empty(), "{issues:?}");
        let c = rs.classify(r"C:\Users\Test\AppData\Local\Temp\a.tmp");
        assert_eq!(c.verdict, Verdict::Safe);
        let c = rs.classify(r"C:\Users\Test\AppData\Local\Temp\Shaders\x.bin");
        assert_eq!(c.verdict, Verdict::Consequences);
        assert_eq!(rs.get(c.rule.unwrap()).unwrap().rule.id, "shader");
        assert_eq!(
            rs.classify(r"C:\Users\Test\AppData\Local\Other").verdict,
            Verdict::Unknown
        );
        assert_eq!(
            rs.classify(r"C:\Users\Test\Documents\a.docx").verdict,
            Verdict::Protected
        );
    }

    #[test]
    fn rule_without_source_is_unknown() {
        let text = GOOD.replacen("source = \"doc\"\n", "", 1);
        let (rs, issues) = set(&text);
        assert!(issues
            .iter()
            .any(|i| i.rule_id == "tmp" && i.problem.contains("source")));
        assert_eq!(
            rs.classify(r"C:\Users\Test\AppData\Local\Temp\a.tmp")
                .verdict,
            Verdict::Unknown
        );
        assert!(!rs.get(0).unwrap().deletable());
    }

    #[test]
    fn rule_without_verified_on_is_unknown() {
        let text = GOOD.replacen("verified_on = \"Windows 11 22631, 2026-10-24\"\n", "", 1);
        let (rs, issues) = set(&text);
        assert!(issues.iter().any(|i| i.problem.contains("verified_on")));
        assert!(!rs.get(0).unwrap().deletable());
    }

    #[test]
    fn rule_cannot_override_blacklist() {
        for path in [
            r"%SystemRoot%\System32\LogFiles",
            r"%ProgramFiles%\Steam\appcache",
            r"%USERPROFILE%\Documents",
            r"%LOCALAPPDATA%",
            r"%SystemDrive%\",
        ] {
            let text = GOOD.replacen(r"%LOCALAPPDATA%\Temp'", &format!("{path}'"), 1);
            let (rs, issues) = set(&text);
            assert!(!issues.is_empty(), "{path}");
            assert!(!rs.get(0).unwrap().deletable(), "{path}");
        }
        // Навіть «🟢» правило всередині System32 класифікується як 🔴.
        let text = GOOD.replacen(r"%LOCALAPPDATA%\Temp'", r"%SystemRoot%\System32\x'", 1);
        let (rs, _) = set(&text);
        assert_eq!(
            rs.classify(r"C:\Windows\System32\x\a").verdict,
            Verdict::Protected
        );
    }

    #[test]
    fn incompatible_method_is_demoted() {
        let text = GOOD.replacen("verdict = \"safe\"", "verdict = \"protected\"", 1);
        let (rs, issues) = set(&text);
        assert!(!issues.is_empty());
        assert!(!rs.get(0).unwrap().deletable());
        let text = GOOD.replacen("method = \"delete\"", "method = \"manual_only\"", 1);
        let (_, issues) = set(&text);
        assert!(issues.iter().any(|i| i.problem.contains("manual_hint")));
    }

    #[test]
    fn unknown_service_is_rejected() {
        let text = GOOD.replacen(
            "method = \"delete\"",
            "method = \"delete\"\nstop_services = [\"WinDefend\"]",
            1,
        );
        let (rs, issues) = set(&text);
        assert!(issues.iter().any(|i| i.problem.contains("WinDefend")));
        assert!(!rs.get(0).unwrap().deletable());
    }

    #[test]
    fn broken_file_reports_issue() {
        let (rs, issues) = set("[[rule]]\nid = 1");
        assert!(rs.rules().is_empty());
        assert_eq!(issues.len(), 1);
    }
}
