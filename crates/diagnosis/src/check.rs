//! «Перевірити мою гру»: за чим судити «було / стало» після головної дії розповіді.
//!
//! Дія проти ривків (мікрофризи, фонові процеси, відеопам'ять) перевіряється за 1% low, решта —
//! за середнім FPS. Порогів тут немає: висновок рахує `history::check` з наявним порогом похибки.

use crate::{Diagnosis, StoryOutcome};
use optimax_core::{Category, NextStep, Text};
use serde::{Deserialize, Serialize};

/// За якою метрикою робити висновок «було / стало».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CheckFocus {
    /// Середній FPS.
    Fps,
    /// 1% low (ривки).
    Lows,
}

/// Як перевіряти головну дію розповіді.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CheckPlan {
    /// Метрика висновку.
    pub focus: CheckFocus,
    /// Варіанти для точної перевірки A/B («як зараз» і після зміни), якщо дію можна перемкнути.
    pub ab: Option<[Text; 2]>,
    /// Порада, яку перевіряють («Що змінено» у «Було / стало»).
    #[serde(default)]
    pub advice: Option<Text>,
}

/// Обрана людиною порада («Спробувати»): id знахідки і, щоб розрізнити знахідки з тим самим id
/// (кілька фонових процесів), — текст поради.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CheckChoice {
    /// Id знахідки.
    pub id: String,
    /// Текст поради; `None` — перша знахідка з цим id.
    #[serde(default)]
    pub advice: Option<Text>,
}

/// План перевірки: `finding = None` — головна дія розповіді (лише «є що покращити» чи «ривки без
/// причини»); `Some(id)` — обрана людиною порада (головна чи з «Інших варіантів»), зокрема коли
/// «усе гаразд». Знахідка шукається за id: у запису «було», заново проаналізованому з «Історії»,
/// порядок знахідок може бути іншим. `None` — перевіряти нічого.
pub fn check_plan(d: &Diagnosis, finding: Option<&CheckChoice>) -> Option<CheckPlan> {
    let story = d.story.as_ref()?;
    let index = match finding {
        None => {
            if !matches!(
                story.outcome,
                StoryOutcome::Fixable | StoryOutcome::Unexplained
            ) {
                return None;
            }
            story.action?
        }
        Some(choice) => {
            if story.outcome == StoryOutcome::Insufficient {
                return None;
            }
            story
                .action
                .iter()
                .chain(&story.others)
                .copied()
                .find(|&i| {
                    d.verdicts.get(i as usize).is_some_and(|v| {
                        v.finding.id == choice.id
                            && (choice.advice.is_none() || v.finding.advice == choice.advice)
                    })
                })?
        }
    };
    let finding = &d.verdicts.get(index as usize)?.finding;
    let focus = match finding.category {
        Category::Stutter | Category::Background | Category::Vram => CheckFocus::Lows,
        _ => CheckFocus::Fps,
    };
    let ab = match &finding.next {
        Some(NextStep::Compare { variants }) => Some(variants.clone()),
        _ => None,
    };
    Some(CheckPlan {
        focus,
        ab,
        advice: finding.advice.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bottleneck, Smoothness, Story, Verdict};
    use optimax_core::{FindingBuilder, Impact};

    fn variants() -> [Text; 2] {
        [Text::new("a"), Text::new("b")]
    }

    /// Підсумок + знахідка `category` з порадою; розповідь із головною дією `action`.
    fn diag(category: Category, outcome: StoryOutcome, action: Option<u32>) -> Diagnosis {
        let summary = FindingBuilder::new("diag", "x", Category::Summary, Impact::Info, 80).build();
        let mut f = FindingBuilder::new("diag", "x", category, Impact::Moderate, 80).build();
        f.id = "finding".into();
        f.advice = Some(Text::new("advice"));
        f.next = Some(NextStep::Compare {
            variants: variants(),
        });
        Diagnosis {
            game: "game.exe".into(),
            stats: None,
            bottleneck: Bottleneck::Gpu,
            verdicts: vec![Verdict::from(summary), Verdict::from(f)],
            chart: None,
            frame_gen: None,
            anatomy: None,
            latency: None,
            view: None,
            story: Some(Story {
                smoothness: Smoothness::Noticeable,
                outcome,
                how: vec![],
                why: vec![],
                why_unknown: false,
                action,
                others: vec![],
            }),
        }
    }

    #[test]
    fn plan_fps_for_gpu_advice() {
        let p = check_plan_main(&diag(Category::Gpu, StoryOutcome::Fixable, Some(1))).unwrap();
        assert_eq!(p.focus, CheckFocus::Fps);
        assert_eq!(p.ab, Some(variants()));
    }

    #[test]
    fn plan_lows_for_stutter_background_vram() {
        for c in [Category::Stutter, Category::Background, Category::Vram] {
            let p = check_plan_main(&diag(c, StoryOutcome::Fixable, Some(1))).unwrap();
            assert_eq!(p.focus, CheckFocus::Lows, "{c:?}");
        }
    }

    #[test]
    fn plan_unexplained_is_lows() {
        let mut d = diag(Category::Stutter, StoryOutcome::Unexplained, Some(1));
        d.verdicts[1].finding.id = "stutter.unknown".into();
        assert_eq!(check_plan_main(&d).unwrap().focus, CheckFocus::Lows);
    }

    #[test]
    fn plan_none_when_fine_or_insufficient() {
        for o in [StoryOutcome::Fine, StoryOutcome::Insufficient] {
            assert_eq!(
                check_plan_main(&diag(Category::Gpu, o, Some(1))),
                None,
                "{o:?}"
            );
        }
    }

    #[test]
    fn plan_none_without_story_or_action() {
        let mut d = diag(Category::Gpu, StoryOutcome::Fixable, None);
        assert_eq!(check_plan_main(&d), None);
        d.story = None;
        assert_eq!(check_plan_main(&d), None);
    }

    #[test]
    fn plan_none_when_action_index_out_of_range() {
        assert_eq!(
            check_plan_main(&diag(Category::Gpu, StoryOutcome::Fixable, Some(7))),
            None
        );
    }

    #[test]
    fn plan_without_next_has_no_ab() {
        let mut d = diag(Category::Gpu, StoryOutcome::Fixable, Some(1));
        d.verdicts[1].finding.next = None;
        assert_eq!(check_plan_main(&d).unwrap().ab, None);
    }

    fn by_id(id: &str) -> CheckChoice {
        CheckChoice {
            id: id.into(),
            advice: None,
        }
    }

    fn check_plan_main(d: &Diagnosis) -> Option<CheckPlan> {
        check_plan(d, None)
    }

    /// `Fine`: головної дії немає, знахідка 1 — серед «Інших варіантів».
    fn fine_with_other() -> Diagnosis {
        let mut d = diag(Category::Gpu, StoryOutcome::Fine, None);
        if let Some(s) = d.story.as_mut() {
            s.others = vec![1];
        }
        d
    }

    #[test]
    fn plan_for_chosen_other_in_fine() {
        let p = check_plan(&fine_with_other(), Some(&by_id("finding"))).unwrap();
        assert_eq!(p.focus, CheckFocus::Fps);
        assert_eq!(p.advice, Some(Text::new("advice")));
        assert_eq!(p.ab, Some(variants()));
    }

    #[test]
    fn plan_by_id_not_index() {
        let mut d = fine_with_other();
        let extra = FindingBuilder::new("diag", "x", Category::Stutter, Impact::Info, 80).build();
        d.verdicts.insert(1, Verdict::from(extra));
        if let Some(s) = d.story.as_mut() {
            s.others = vec![2];
        }
        assert_eq!(
            check_plan(&d, Some(&by_id("finding"))).unwrap().focus,
            CheckFocus::Fps
        );
    }

    #[test]
    fn plan_unknown_or_unlisted_id_is_none() {
        let d = fine_with_other();
        assert_eq!(check_plan(&d, Some(&by_id("nope"))), None);
        let mut d = d;
        if let Some(s) = d.story.as_mut() {
            s.others.clear();
        }
        assert_eq!(check_plan(&d, Some(&by_id("finding"))), None);
    }

    #[test]
    fn plan_chosen_in_insufficient_is_none() {
        let mut d = fine_with_other();
        if let Some(s) = d.story.as_mut() {
            s.outcome = StoryOutcome::Insufficient;
        }
        assert_eq!(check_plan(&d, Some(&by_id("finding"))), None);
    }

    #[test]
    fn plan_main_has_advice() {
        let p = check_plan_main(&diag(Category::Gpu, StoryOutcome::Fixable, Some(1))).unwrap();
        assert_eq!(p.advice, Some(Text::new("advice")));
    }

    #[test]
    fn plan_tells_apart_same_id_by_advice() {
        // `background` буває кілька разів з тим самим id — різняться порадою (ім'я процесу).
        let mut d = fine_with_other();
        let mut second = d.verdicts[1].finding.clone();
        second.category = Category::Background;
        second.advice = Some(Text::new("advice").arg("name", "discord.exe"));
        d.verdicts.push(Verdict::from(second));
        if let Some(s) = d.story.as_mut() {
            s.others = vec![1, 2];
        }
        let choice = CheckChoice {
            id: "finding".into(),
            advice: Some(Text::new("advice").arg("name", "discord.exe")),
        };
        let p = check_plan(&d, Some(&choice)).unwrap();
        assert_eq!(p.focus, CheckFocus::Lows);
        assert_eq!(p.advice, choice.advice);
    }
}
