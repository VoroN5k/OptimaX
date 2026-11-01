//! Розкладка пам'яті по каналах: один модуль, усі модулі в одному каналі, різний обсяг у
//! каналах, змішані модулі. Чисті функції без системних викликів.
//!
//! Канал визначаємо лише за явним словом `CHANNEL` у підписах SMBIOS (Bank Locator і Device
//! Locator), напр. `P0 CHANNEL A` / `DIMM 1` (AMD AGESA) або `BANK 0` /
//! `Controller0-ChannelA-DIMM0` (Intel). Інші схеми підписів (`DIMM_A1`, `DIMM 0`) різні BIOS
//! трактують по-різному, тому для них канал вважається невідомим і вердикту немає.

use crate::model::{MemoryModule, MemoryType};
use crate::rules::draft;
use optimax_core::{Category, Evidence, Finding, Impact, Text};

/// Канал пам'яті: контролер / сокет (якщо підписаний) і літера каналу.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemoryChannel {
    /// Номер контролера (`Controller1`) або сокета (`P0`), якщо є в підписі.
    pub controller: Option<u8>,
    /// Літера каналу, `A`…`H`.
    pub letter: char,
}

/// Канал модуля за підписами SMBIOS. `None` — підпис не містить однозначного каналу.
pub fn channel_of(bank: &str, locator: &str) -> Option<MemoryChannel> {
    let text = format!("{bank} {locator}").to_ascii_uppercase();
    let mut letters = letters_after(&text, "CHANNEL");
    let letter = letters.next()?;
    // Різні літери в банку й слоті — суперечність, не вгадуємо.
    if letters.any(|l| l != letter) {
        return None;
    }
    let controller = number_after(&text, "CONTROLLER").or_else(|| socket(&text));
    Some(MemoryChannel { controller, letter })
}

/// Літери каналу після кожного входження `word` (через пробіл, `_`, `-` або `:`).
fn letters_after<'a>(text: &'a str, word: &'a str) -> impl Iterator<Item = char> + 'a {
    text.match_indices(word).filter_map(move |(i, _)| {
        let mut rest = text[i + word.len()..]
            .trim_start_matches([' ', '_', '-', ':'])
            .chars();
        let letter = rest.next().filter(|c| ('A'..='H').contains(c))?;
        // `CHANNELA-DIMM0` так, `CHANNEL AB` чи `CHANNELS` — ні.
        match rest.next() {
            Some(c) if c.is_ascii_alphabetic() => None,
            _ => Some(letter),
        }
    })
}

/// Число одразу після `word`, напр. `CONTROLLER1` → 1.
fn number_after(text: &str, word: &str) -> Option<u8> {
    let i = text.find(word)? + word.len();
    let digits: String = text[i..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Сокет як окреме слово `P<цифри>`, напр. `P0 CHANNEL A`.
fn socket(text: &str) -> Option<u8> {
    text.split([' ', '_', '-'])
        .find_map(|t| {
            t.strip_prefix('P')
                .filter(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
        })
        .and_then(|d| d.parse().ok())
}

fn gb(mib: u64) -> String {
    let tenths = (mib * 10 + 512) / 1024;
    if tenths.is_multiple_of(10) {
        (tenths / 10).to_string()
    } else {
        format!("{},{}", tenths / 10, tenths % 10)
    }
}

/// Розкладка пам'яті: канали й однаковість модулів.
///
/// `slots` — загальна кількість слотів за SMBIOS (0 — невідомо). Для розпаяної LPDDR і
/// невідомих типів пам'яті канальний вердикт не робиться (у них інша будова каналів).
pub fn check_memory_layout(modules: &[MemoryModule], slots: u32) -> Vec<Finding> {
    let socketed =
        |t: MemoryType| matches!(t, MemoryType::Ddr3 | MemoryType::Ddr4 | MemoryType::Ddr5);
    if modules.is_empty() || !modules.iter().all(|m| socketed(m.memory_type)) {
        return Vec::new();
    }
    let installed = modules.len();
    let slots_arg = if slots as usize >= installed {
        slots.to_string()
    } else {
        "?".into()
    };
    let mut out = Vec::new();

    if installed == 1 {
        let m = &modules[0];
        out.push(
            draft("ram.single_module", Category::Memory, Impact::High, 95)
                .arg("size_gb", gb(m.size_mib))
                .arg("slots", &slots_arg)
                .advice()
                .evidence(
                    Text::new("audit.ev.slots"),
                    Text::new("audit.ev.slots_value")
                        .arg("installed", installed)
                        .arg("slots", &slots_arg),
                )
                .build(),
        );
        return out;
    }

    let channels: Option<Vec<MemoryChannel>> = modules
        .iter()
        .map(|m| channel_of(&m.bank, &m.locator))
        .collect();
    match channels {
        None => {
            let evidence = modules.iter().map(|m| {
                Evidence::new(
                    Text::new("audit.ev.module_label"),
                    Text::raw(m.slot_label()),
                )
            });
            out.push(
                draft("ram.channels_unknown", Category::Memory, Impact::Info, 100)
                    .arg("count", installed)
                    .evidence_list(evidence)
                    .build(),
            );
        }
        Some(channels) => out.push(channel_finding(modules, &channels, &slots_arg)),
    }

    if let Some(f) = check_mixed(modules) {
        out.push(f);
    }
    out
}

/// Вердикт за відомими каналами всіх модулів.
fn channel_finding(modules: &[MemoryModule], channels: &[MemoryChannel], slots: &str) -> Finding {
    // Обсяг і слоти кожного каналу, у порядку контролер → літера.
    let mut per_channel: Vec<(MemoryChannel, u64, Vec<String>)> = Vec::new();
    for (m, &ch) in modules.iter().zip(channels) {
        match per_channel.iter_mut().find(|(c, _, _)| *c == ch) {
            Some((_, size, labels)) => {
                *size += m.size_mib;
                labels.push(m.slot_label());
            }
            None => per_channel.push((ch, m.size_mib, vec![m.slot_label()])),
        }
    }
    per_channel.sort_by_key(|(c, _, _)| *c);
    let many_controllers = {
        let mut ctrls: Vec<_> = per_channel.iter().map(|(c, _, _)| c.controller).collect();
        ctrls.dedup();
        ctrls.len() > 1
    };
    let name = |c: &MemoryChannel| match (many_controllers, c.controller) {
        (true, Some(n)) => format!("{n}-{}", c.letter),
        _ => c.letter.to_string(),
    };
    let evidence: Vec<Evidence> = per_channel
        .iter()
        .map(|(c, size, labels)| {
            Evidence::new(
                Text::new("audit.ev.channel").arg("channel", name(c)),
                Text::new("audit.ev.channel_value")
                    .arg("size_gb", gb(*size))
                    .arg("slots", labels.join(", ")),
            )
        })
        .collect();

    let count = per_channel.len();
    let builder = if count == 1 {
        draft("ram.single_channel", Category::Memory, Impact::High, 80)
            .arg("count", modules.len())
            .arg("channel", name(&per_channel[0].0))
            .arg("slots", slots)
            .advice()
    } else {
        let min = per_channel.iter().map(|(_, s, _)| *s).min().unwrap_or(0);
        let max = per_channel.iter().map(|(_, s, _)| *s).max().unwrap_or(0);
        if min == max {
            draft("ram.multi_channel", Category::Memory, Impact::Info, 80)
                .arg("channels", count)
                .arg("size_gb", gb(min))
        } else {
            draft("ram.asymmetric", Category::Memory, Impact::Moderate, 70)
                .arg("channels", count)
                .arg("min_gb", gb(min))
                .arg("max_gb", gb(max))
                .advice()
        }
    };
    builder.evidence_list(evidence).build()
}

/// Змішані модулі: різний обсяг, заявлена частота або партномер.
fn check_mixed(modules: &[MemoryModule]) -> Option<Finding> {
    let differs = |f: &dyn Fn(&MemoryModule) -> String| {
        let mut values: Vec<String> = modules.iter().map(f).filter(|v| !v.is_empty()).collect();
        values.sort();
        values.dedup();
        values.len() > 1
    };
    let size = differs(&|m| m.size_mib.to_string());
    let speed = differs(&|m| m.rated_mts.map(|v| v.to_string()).unwrap_or_default());
    let part = differs(&|m| m.part_number.clone());
    if !(size || speed || part) {
        return None;
    }
    let what: Vec<&str> = [
        (size, "audit.ram.mixed.size"),
        (speed, "audit.ram.mixed.speed"),
        (part, "audit.ram.mixed.part"),
    ]
    .into_iter()
    .filter_map(|(on, key)| on.then_some(key))
    .collect();
    let evidence = modules.iter().map(|m| {
        Evidence::new(
            Text::raw(m.slot_label()),
            Text::new("audit.ev.mixed_value")
                .arg("size_gb", gb(m.size_mib))
                .arg(
                    "rated",
                    m.rated_mts
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".into()),
                )
                .arg(
                    "part",
                    if m.part_number.is_empty() {
                        "—"
                    } else {
                        &m.part_number
                    },
                ),
        )
    });
    let mut b = draft("ram.mixed", Category::Memory, Impact::Info, 90).advice();
    for (i, key) in what.iter().enumerate() {
        b = b.evidence(
            Text::new("audit.ev.mixed_reason").arg("n", i + 1),
            Text::new(*key),
        );
    }
    Some(b.evidence_list(evidence).build())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(controller: Option<u8>, letter: char) -> Option<MemoryChannel> {
        Some(MemoryChannel { controller, letter })
    }

    fn module(bank: &str, locator: &str, size_gb: u64, part: &str) -> MemoryModule {
        MemoryModule {
            locator: locator.into(),
            bank: bank.into(),
            size_mib: size_gb * 1024,
            memory_type: MemoryType::Ddr4,
            rated_mts: Some(3600),
            configured_mts: Some(3600),
            manufacturer: "G.Skill".into(),
            part_number: part.into(),
        }
    }

    const KIT: &str = "F4-3600C18-16GTZN";

    #[test]
    fn channel_from_known_label_schemes() {
        // AMD AGESA (ПК користувача: Ryzen 7 5800X).
        assert_eq!(channel_of("P0 CHANNEL A", "DIMM 1"), ch(Some(0), 'A'));
        assert_eq!(channel_of("P0 CHANNEL B", "DIMM 1"), ch(Some(0), 'B'));
        // Intel: канал у Device Locator.
        assert_eq!(channel_of("BANK 0", "ChannelA-DIMM0"), ch(None, 'A'));
        assert_eq!(channel_of("BANK 2", "ChannelB-DIMM1"), ch(None, 'B'));
        assert_eq!(
            channel_of("BANK 0", "Controller1-ChannelA-DIMM1"),
            ch(Some(1), 'A')
        );
        assert_eq!(channel_of("", "CHANNEL_C DIMM 0"), ch(None, 'C'));
    }

    #[test]
    fn ambiguous_labels_give_no_channel() {
        assert_eq!(channel_of("BANK 0", "DIMM_A1"), None);
        assert_eq!(channel_of("", "DIMM 0"), None);
        assert_eq!(channel_of("", ""), None);
        assert_eq!(channel_of("CHANNELS", "DIMM 1"), None);
        assert_eq!(channel_of("CHANNEL AB", "DIMM 1"), None);
        assert_eq!(channel_of("CHANNEL Z", "DIMM 1"), None);
        // Банк і слот суперечать одне одному.
        assert_eq!(channel_of("P0 CHANNEL A", "ChannelB-DIMM0"), None);
    }

    #[test]
    fn user_pc_is_dual_channel_and_not_mixed() {
        let f = check_memory_layout(
            &[
                module("P0 CHANNEL A", "DIMM 1", 16, KIT),
                module("P0 CHANNEL B", "DIMM 1", 16, KIT),
            ],
            4,
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "ram.multi_channel");
        assert_eq!(f[0].evidence.len(), 2);
    }

    #[test]
    fn single_module_is_single_channel_without_labels() {
        let f = check_memory_layout(&[module("BANK 0", "DIMM_A1", 16, KIT)], 4);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "ram.single_module");
        assert_eq!(f[0].impact, Impact::High);
        assert!(f[0].advice.is_some());
    }

    #[test]
    fn two_modules_in_one_channel() {
        let f = check_memory_layout(
            &[
                module("P0 CHANNEL A", "DIMM 0", 8, KIT),
                module("P0 CHANNEL A", "DIMM 1", 8, KIT),
            ],
            4,
        );
        assert_eq!(f[0].id, "ram.single_channel");
        assert_eq!(f[0].evidence.len(), 1);
    }

    #[test]
    fn intel_two_controllers_are_two_channels() {
        let f = check_memory_layout(
            &[
                module("BANK 0", "Controller0-ChannelA-DIMM1", 16, KIT),
                module("BANK 0", "Controller1-ChannelA-DIMM1", 16, KIT),
            ],
            4,
        );
        assert_eq!(f[0].id, "ram.multi_channel");
    }

    #[test]
    fn unequal_channels_are_asymmetric_and_mixed() {
        let f = check_memory_layout(
            &[
                module("BANK 0", "ChannelA-DIMM0", 16, KIT),
                module("BANK 1", "ChannelB-DIMM0", 8, "F4-3200C16-8GVKB"),
            ],
            2,
        );
        let ids: Vec<_> = f.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["ram.asymmetric", "ram.mixed"]);
        assert_eq!(f[0].impact, Impact::Moderate);
    }

    #[test]
    fn unknown_labels_give_no_channel_verdict() {
        let f = check_memory_layout(
            &[
                module("BANK 0", "DIMM_A1", 16, KIT),
                module("BANK 1", "DIMM_B1", 16, KIT),
            ],
            4,
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "ram.channels_unknown");
        assert_eq!(f[0].impact, Impact::Info);
    }

    #[test]
    fn mixed_part_numbers_only() {
        let f = check_memory_layout(
            &[
                module("P0 CHANNEL A", "DIMM 1", 16, KIT),
                module("P0 CHANNEL B", "DIMM 1", 16, "CMK16GX4M1Z3600C18"),
            ],
            4,
        );
        let ids: Vec<_> = f.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["ram.multi_channel", "ram.mixed"]);
        // Причина одна (партномер) + рядок на кожен модуль.
        assert_eq!(f[1].evidence.len(), 3);
    }

    #[test]
    fn soldered_lpddr_and_empty_are_skipped() {
        let mut m = module("BANK 0", "ChannelA-DIMM0", 8, "");
        m.memory_type = MemoryType::Lpddr5;
        assert!(check_memory_layout(&[m], 0).is_empty());
        assert!(check_memory_layout(&[], 4).is_empty());
    }

    #[test]
    fn gb_formatting() {
        assert_eq!(gb(16384), "16");
        assert_eq!(gb(12288), "12");
        assert_eq!(gb(6144 + 512), "6,5");
    }
}
