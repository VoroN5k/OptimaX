//! Звіт порівняння в Markdown — щоб можна було викласти на форумі.

use crate::{letter, Comparison, Estimate, Outcome, Report, Variant};
use optimax_core::{Catalog, Text};

fn signed(v: f64) -> String {
    format!("{v:+.1}")
}

/// Середнє ± стандартне відхилення значень прогонів варіанта (невиключених).
fn spread(report: &Report, v: Variant, value: fn(&crate::RunSummary) -> f64) -> String {
    let vals: Vec<f64> = report
        .runs
        .iter()
        .filter(|r| !r.excluded && r.variant == v)
        .map(value)
        .collect();
    if vals.is_empty() {
        return "—".into();
    }
    let n = vals.len() as f64;
    let m = vals.iter().sum::<f64>() / n;
    let sd = if vals.len() > 1 {
        (vals.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    } else {
        0.0
    };
    format!("{m:.1} ± {sd:.1}")
}

fn change(c: &Catalog, e: Option<&Estimate>) -> String {
    e.map_or_else(
        || "—".into(),
        |e| {
            c.render(
                &Text::new("exp.md.change")
                    .arg("delta", signed(e.delta_pct))
                    .arg("lo", signed(e.ci_low_pct))
                    .arg("hi", signed(e.ci_high_pct)),
            )
        },
    )
}

fn outcome_key(o: Outcome) -> &'static str {
    match o {
        Outcome::Better => "exp.md.outcome.better",
        Outcome::Worse => "exp.md.outcome.worse",
        Outcome::NoEffect => "exp.md.outcome.no_effect",
        Outcome::Unclear => "exp.md.outcome.unclear",
    }
}

fn name(names: &[String], v: Variant) -> String {
    names
        .get(usize::from(v))
        .cloned()
        .unwrap_or_else(|| letter(v).to_string())
}

fn verdict_line(c: &Catalog, cmp: &Comparison, names: &[String]) -> String {
    let key = match cmp.verdict {
        Outcome::Better => "exp.md.verdict.better",
        Outcome::Worse => "exp.md.verdict.worse",
        Outcome::NoEffect => "exp.md.verdict.no_effect",
        Outcome::Unclear => "exp.md.verdict.unclear",
    };
    c.render(
        &Text::new(key)
            .arg("a", name(names, 0))
            .arg("b", name(names, cmp.variant)),
    )
}

/// Звіт у Markdown. `names` — назви варіантів за номером, `title` — що порівнювали.
pub fn render_markdown(
    report: &Report,
    c: &Catalog,
    names: &[String],
    title: &str,
    game: &str,
    seconds: u32,
) -> String {
    let order: String = report.runs.iter().map(|r| letter(r.variant)).collect();
    let legend = (0..report.variants)
        .map(|v| format!("{} — {}", letter(v), name(names, v)))
        .collect::<Vec<_>>()
        .join(", ");
    let per_arm = (0..report.variants)
        .map(|v| report.runs_of(v))
        .min()
        .unwrap_or(0);
    let mut md = vec![
        c.render(&Text::new("exp.md.title").arg("title", title)),
        String::new(),
        c.render(
            &Text::new("exp.md.setup")
                .arg("game", game)
                .arg("runs", per_arm)
                .arg("seconds", seconds)
                .arg("order", order)
                .arg("legend", legend),
        ),
        String::new(),
        c.render(&Text::new("exp.md.header")),
        "|---|---|---|---|---|---|".into(),
    ];
    for v in 0..report.variants {
        let cmp = report.comparison(v);
        let outcome = cmp.map_or_else(
            || c.render(&Text::new("exp.md.base")),
            |x| c.render(&Text::new(outcome_key(x.verdict))),
        );
        md.push(format!(
            "| {} — {} | {} | {} | {} | {} | {outcome} |",
            letter(v),
            name(names, v),
            spread(report, v, |r| r.avg_fps),
            spread(report, v, |r| r.low_1pct_fps),
            change(c, cmp.and_then(|x| x.avg_fps.as_ref())),
            change(c, cmp.and_then(|x| x.low_1pct.as_ref())),
        ));
    }
    md.push(String::new());
    for cmp in &report.comparisons {
        md.push(verdict_line(c, cmp, names));
    }
    for w in &report.warnings {
        md.push(format!("> ⚠ {}", c.render(w)));
    }
    md.push(String::new());
    md.push(
        c.render(
            &Text::new("exp.md.method")
                .arg("conf", format!("{:.1}", report.confidence_pct))
                .arg("min", format!("{}", report.margin_avg_pct)),
        ),
    );
    md.join("\n")
}
