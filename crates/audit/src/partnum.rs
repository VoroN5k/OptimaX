//! Частота набору пам'яті за партномером (профіль XMP/EXPO, який Windows не бачить).
//!
//! Лише формати, звірені з офіційними джерелами виробника; невідомий формат → `None`:
//! - **G.Skill** `F<покоління>-<MT/s><C|J><таймінги>…`: специфікації на gskill.com, напр.
//!   F4-3600C18D-32GTZN — «DDR4-3600 CL18», SPD 2133; F5-6000J3038F16GX2-TZ5N — «DDR5-6000
//!   CL30», SPD 4800. Партномер модуля в SPD має той самий початок (на ПК розробника:
//!   `F4-3600C18-16GTZN` з набору F4-3600C18D-32GTZN).
//! - **Corsair** `CM<лінійка><обсяг>GX<покоління>M<модулів><версія><MT/s>C<CL>…`: «How to read
//!   the CORSAIR memory part number» (corsair.com) — «3200: The number of megatransfers per
//!   second when the overclock profile is enabled». Чи записує Corsair цей номер у SPD
//!   модуля, не перевірено: якщо там інший рядок, він просто не розпізнається.
//!
//! Kingston FURY (`KF432…`) свідомо не розбираємо: офіційний декодер не вдалося прочитати, а
//! двозначне поле частоти не завжди × 100 (напр. `26` — це 2666 MT/s).

use crate::model::MemoryType;

/// Частота профілю набору, MT/s, якщо партномер має перевірений формат і збігається з типом
/// пам'яті модуля.
pub fn kit_speed_mts(part_number: &str, memory_type: MemoryType) -> Option<u32> {
    let part = part_number.trim().to_ascii_uppercase();
    let (generation, speed) = gskill(&part).or_else(|| corsair(&part))?;
    let (expected, range) = match memory_type {
        MemoryType::Ddr3 => (3, 800..=3200),
        MemoryType::Ddr4 => (4, 1600..=6000),
        MemoryType::Ddr5 => (5, 3200..=12_000),
        _ => return None,
    };
    (generation == expected && range.contains(&speed)).then_some(speed)
}

/// Цифри на початку рядка (від `min` до `max` штук) і решта рядка.
fn digits(s: &str, min: usize, max: usize) -> Option<(u32, &str)> {
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    if n < min || n > max {
        return None;
    }
    Some((s[..n].parse().ok()?, &s[n..]))
}

/// `F4-3600C18…` / `F5-6000J30…` → (покоління, MT/s).
fn gskill(s: &str) -> Option<(u8, u32)> {
    let rest = s.strip_prefix('F')?;
    let (generation, rest) = digits(rest, 1, 1)?;
    let rest = rest.strip_prefix('-')?;
    let (speed, rest) = digits(rest, 4, 5)?;
    let rest = rest.strip_prefix(['C', 'J'])?;
    // CL (DDR4: `C18`) або CL і tRCD разом (DDR5: `J3038` = 30-38).
    digits(rest, 1, 4)?;
    Some((u8::try_from(generation).ok()?, speed))
}

/// `CMK32GX4M2Z3200C16W` → (покоління, MT/s).
fn corsair(s: &str) -> Option<(u8, u32)> {
    let rest = s.strip_prefix("CM")?;
    // Лінійка: одна чи кілька літер (K, W, T, H …).
    let line = rest.bytes().take_while(u8::is_ascii_uppercase).count();
    if line == 0 {
        return None;
    }
    let (_capacity, rest) = digits(&rest[line..], 1, 4)?;
    let rest = rest.strip_prefix("GX")?;
    let (generation, rest) = digits(rest, 1, 1)?;
    let rest = rest.strip_prefix('M')?;
    let (_modules, rest) = digits(rest, 1, 2)?;
    // Версія моделі: одна літера.
    let mut chars = rest.chars();
    if !chars.next()?.is_ascii_uppercase() {
        return None;
    }
    let (speed, rest) = digits(chars.as_str(), 4, 5)?;
    let rest = rest.strip_prefix('C')?;
    digits(rest, 1, 2)?;
    Some((u8::try_from(generation).ok()?, speed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use MemoryType::{Ddr3, Ddr4, Ddr5, Lpddr5};

    #[test]
    fn gskill_formats() {
        // SPD модуля на ПК розробника.
        assert_eq!(kit_speed_mts("F4-3600C18-16GTZN", Ddr4), Some(3600));
        // Номери наборів зі специфікацій gskill.com.
        assert_eq!(kit_speed_mts("F4-3600C18D-32GTZN", Ddr4), Some(3600));
        assert_eq!(kit_speed_mts("F5-6000J3038F16GX2-TZ5N", Ddr5), Some(6000));
        assert_eq!(kit_speed_mts("f4-3200c16-8gvkb ", Ddr4), Some(3200));
        assert_eq!(kit_speed_mts("F3-1600C9D-8GXM", Ddr3), Some(1600));
    }

    #[test]
    fn corsair_formats() {
        // Приклад з офіційного гайда Corsair.
        assert_eq!(kit_speed_mts("CMK32GX4M2Z3200C16W", Ddr4), Some(3200));
        assert_eq!(kit_speed_mts("CMK16GX4M2B3200C16", Ddr4), Some(3200));
        assert_eq!(kit_speed_mts("CMK32GX5M2B5600C36", Ddr5), Some(5600));
    }

    #[test]
    fn generation_must_match_module_type() {
        assert_eq!(kit_speed_mts("F4-3600C18-16GTZN", Ddr5), None);
        assert_eq!(kit_speed_mts("CMK32GX5M2B5600C36", Ddr4), None);
        assert_eq!(kit_speed_mts("F5-6000J3038F16GX2-TZ5N", Lpddr5), None);
    }

    #[test]
    fn unknown_or_implausible_give_none() {
        for p in [
            "",
            "KF432C16BB/16",
            "M378A1K43CB2-CTD",
            "HMA81GU6CJR8N-VK",
            "F4-36C18",
            "F4-3600X18",
            "F4-3600C",
            "CMK32GX4M2Z32C16",
            "CM32GX4M2Z3200C16",
            "CMK32GX4M2Z3200",
            "Unknown",
        ] {
            assert_eq!(kit_speed_mts(p, Ddr4), None, "{p}");
        }
        // Частота поза діапазоном покоління.
        assert_eq!(kit_speed_mts("F4-9600C18-16GTZN", Ddr4), None);
    }
}
