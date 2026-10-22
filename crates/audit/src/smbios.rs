//! Розбір сирої таблиці SMBIOS (`GetSystemFirmwareTable('RSMB')`). Чиста логіка без WinAPI.
//!
//! Поля за специфікацією DMTF SMBIOS 3.x: Type 0 (BIOS) і Type 17 (Memory Device).

use crate::model::{BiosInfo, FirmwareInfo, MemoryModule, MemoryType};
use optimax_core::{CivilDate, CoreError};

/// Розмір заголовка `RawSMBIOSData`, який Windows додає перед таблицею.
const RAW_HEADER_LEN: usize = 8;
const TYPE_BIOS: u8 = 0;
const TYPE_MEMORY_DEVICE: u8 = 17;
const TYPE_END: u8 = 127;

/// Одна структура SMBIOS: відформатована частина + рядки.
struct Structure<'a> {
    kind: u8,
    formatted: &'a [u8],
    strings: Vec<&'a [u8]>,
}

impl Structure<'_> {
    fn byte(&self, off: usize) -> Option<u8> {
        self.formatted.get(off).copied()
    }

    fn word(&self, off: usize) -> Option<u16> {
        let b = self.formatted.get(off..off + 2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    fn dword(&self, off: usize) -> Option<u32> {
        let b = self.formatted.get(off..off + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Рядок за індексом із байта `off` (індекси з 1, 0 — рядка немає).
    fn string(&self, off: usize) -> String {
        let idx = usize::from(self.byte(off).unwrap_or(0));
        if idx == 0 {
            return String::new();
        }
        self.strings
            .get(idx - 1)
            .map(|s| String::from_utf8_lossy(s).trim().to_string())
            .unwrap_or_default()
    }
}

/// Ділить таблицю на структури. Обрізана структура в кінці ігнорується.
fn structures(table: &[u8]) -> Vec<Structure<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= table.len() {
        let kind = table[pos];
        let len = usize::from(table[pos + 1]);
        if len < 4 || pos + len > table.len() {
            break;
        }
        let formatted = &table[pos..pos + len];
        // Рядки: послідовність C-рядків, кінець — подвійний нуль.
        let mut cursor = pos + len;
        let mut strings = Vec::new();
        let Some(rest) = table.get(cursor..) else {
            break;
        };
        if rest.starts_with(&[0, 0]) {
            cursor += 2;
        } else {
            let mut terminated = false;
            while let Some(rel_end) = table[cursor..].iter().position(|&b| b == 0) {
                strings.push(&table[cursor..cursor + rel_end]);
                cursor += rel_end + 1;
                if table.get(cursor) == Some(&0) {
                    cursor += 1;
                    terminated = true;
                    break;
                }
            }
            if !terminated {
                break;
            }
        }
        out.push(Structure {
            kind,
            formatted,
            strings,
        });
        if kind == TYPE_END {
            break;
        }
        pos = cursor;
    }
    out
}

fn parse_bios(s: &Structure<'_>) -> BiosInfo {
    BiosInfo {
        vendor: s.string(0x04),
        version: s.string(0x05),
        release_date: CivilDate::parse_mdy(&s.string(0x08)),
    }
}

/// Розмір модуля в МіБ. `None` — слот порожній або розмір невідомий.
fn module_size_mib(s: &Structure<'_>) -> Option<u64> {
    match s.word(0x0C)? {
        0 | 0xFFFF => None,
        // 0x7FFF: розмір у полі Extended Size (МіБ, біти 0–30).
        0x7FFF => s
            .dword(0x1C)
            .map(|v| u64::from(v & 0x7FFF_FFFF))
            .filter(|&v| v > 0),
        // Біт 15: одиниця — КіБ.
        v if v & 0x8000 != 0 => Some(u64::from(v & 0x7FFF) / 1024),
        v => Some(u64::from(v)),
    }
}

/// Швидкість із 16-бітного поля з переходом на 32-бітне розширене при 0xFFFF.
fn speed(s: &Structure<'_>, off: usize, ext_off: usize) -> Option<u32> {
    match s.word(off)? {
        0 => None,
        0xFFFF => s.dword(ext_off).map(|v| v & 0x7FFF_FFFF).filter(|&v| v > 0),
        v => Some(u32::from(v)),
    }
}

fn parse_memory(s: &Structure<'_>) -> Option<MemoryModule> {
    let size_mib = module_size_mib(s)?;
    Some(MemoryModule {
        locator: s.string(0x10),
        size_mib,
        memory_type: MemoryType::from_smbios(s.byte(0x12).unwrap_or(0)),
        rated_mts: speed(s, 0x15, 0x54),
        configured_mts: speed(s, 0x20, 0x58),
        manufacturer: s.string(0x17),
        part_number: s.string(0x1A),
    })
}

/// Розбирає буфер, повернутий `GetSystemFirmwareTable('RSMB', 0)`.
pub fn parse_raw_smbios(raw: &[u8]) -> Result<FirmwareInfo, CoreError> {
    let len_bytes = raw
        .get(4..RAW_HEADER_LEN)
        .ok_or_else(|| CoreError::InvalidData("SMBIOS: короткий заголовок".into()))?;
    let declared =
        u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
    let table = raw
        .get(RAW_HEADER_LEN..)
        .ok_or_else(|| CoreError::InvalidData("SMBIOS: немає таблиці".into()))?;
    let table = &table[..declared.min(table.len())];

    let mut info = FirmwareInfo::default();
    for s in structures(table) {
        match s.kind {
            TYPE_BIOS if info.bios.is_none() => info.bios = Some(parse_bios(&s)),
            TYPE_MEMORY_DEVICE => info.memory.extend(parse_memory(&s)),
            _ => {}
        }
    }
    Ok(info)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Збирає структуру SMBIOS: тип, відформатована частина (без заголовка), рядки.
    pub(crate) fn structure(kind: u8, body: &[u8], strings: &[&str]) -> Vec<u8> {
        let mut out = vec![kind, u8::try_from(body.len() + 4).unwrap(), 0x00, 0x10];
        out.extend_from_slice(body);
        if strings.is_empty() {
            out.extend_from_slice(&[0, 0]);
        } else {
            for s in strings {
                out.extend_from_slice(s.as_bytes());
                out.push(0);
            }
            out.push(0);
        }
        out
    }

    /// Тіло Type 17 (від зсуву 0x04 до 0x5C), розмір 0x5C байт загалом.
    pub(crate) fn memory_body(size_mib: u16, mem_type: u8, rated: u16, configured: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x5C - 4];
        let put16 = |b: &mut Vec<u8>, off: usize, v: u16| {
            b[off - 4..off - 2].copy_from_slice(&v.to_le_bytes())
        };
        put16(&mut b, 0x0C, size_mib);
        b[0x10 - 4] = 1; // Device Locator → рядок 1
        b[0x12 - 4] = mem_type;
        put16(&mut b, 0x15, rated);
        b[0x17 - 4] = 2; // Manufacturer → рядок 2
        b[0x1A - 4] = 3; // Part Number → рядок 3
        put16(&mut b, 0x20, configured);
        b
    }

    pub(crate) fn raw(table: Vec<u8>) -> Vec<u8> {
        let mut out = vec![0, 3, 6, 0];
        out.extend_from_slice(&u32::try_from(table.len()).unwrap().to_le_bytes());
        out.extend(table);
        out
    }

    fn bios_body() -> Vec<u8> {
        // 0x04 vendor=1, 0x05 version=2, 0x06-0x07 segment, 0x08 date=3, 0x09 rom size … до 0x12.
        let mut b = vec![0u8; 0x12 - 4];
        b[0] = 1;
        b[1] = 2;
        b[4] = 3;
        b
    }

    #[test]
    fn parses_bios_and_memory() {
        let mut table = structure(
            0,
            &bios_body(),
            &["American Megatrends", "F12", "05/14/2024"],
        );
        table.extend(structure(
            17,
            &memory_body(16384, 0x1A, 2133, 2133),
            &["DIMM_A2", "G.Skill", "F4-3600C16-16GVKC"],
        ));
        // Порожній слот.
        table.extend(structure(
            17,
            &memory_body(0, 0x02, 0, 0),
            &["DIMM_A1", "", ""],
        ));
        table.extend(structure(127, &[], &[]));
        let info = parse_raw_smbios(&raw(table)).unwrap();

        let bios = info.bios.unwrap();
        assert_eq!(bios.vendor, "American Megatrends");
        assert_eq!(bios.version, "F12");
        assert_eq!(bios.release_date, CivilDate::new(2024, 5, 14));

        assert_eq!(info.memory.len(), 1);
        let m = &info.memory[0];
        assert_eq!(m.locator, "DIMM_A2");
        assert_eq!(m.size_mib, 16384);
        assert_eq!(m.memory_type, MemoryType::Ddr4);
        assert_eq!(m.rated_mts, Some(2133));
        assert_eq!(m.configured_mts, Some(2133));
        assert_eq!(m.part_number, "F4-3600C16-16GVKC");
    }

    #[test]
    fn extended_size_and_speed() {
        let mut body = memory_body(0x7FFF, 0x22, 0xFFFF, 0xFFFF);
        body[0x1C - 4..0x20 - 4].copy_from_slice(&65536u32.to_le_bytes());
        body[0x54 - 4..0x58 - 4].copy_from_slice(&70000u32.to_le_bytes());
        body[0x58 - 4..0x5C - 4].copy_from_slice(&68000u32.to_le_bytes());
        let info = parse_raw_smbios(&raw(structure(17, &body, &["A", "B", "C"]))).unwrap();
        let m = &info.memory[0];
        assert_eq!(m.size_mib, 65536);
        assert_eq!(m.rated_mts, Some(70000));
        assert_eq!(m.configured_mts, Some(68000));
    }

    #[test]
    fn old_short_type17_has_no_configured_speed() {
        // SMBIOS 2.3: структура закінчується на 0x1B, поля 0x20 немає.
        let body = memory_body(8192, 0x18, 1600, 0)[..0x1B - 4].to_vec();
        let info = parse_raw_smbios(&raw(structure(17, &body, &["A", "B", "C"]))).unwrap();
        assert_eq!(info.memory[0].rated_mts, Some(1600));
        assert_eq!(info.memory[0].configured_mts, None);
    }

    #[test]
    fn truncated_and_garbage_input_do_not_panic() {
        assert!(parse_raw_smbios(&[1, 2]).is_err());
        let mut table = structure(17, &memory_body(4096, 0x1A, 2400, 2400), &["A", "B", "C"]);
        table.truncate(table.len() - 3);
        let info = parse_raw_smbios(&raw(table)).unwrap();
        assert!(info.memory.is_empty());
        let _ = parse_raw_smbios(&raw(vec![0xFF; 64]));
        let _ = parse_raw_smbios(&raw(vec![17, 2, 0, 0, 0, 0]));
    }
}
