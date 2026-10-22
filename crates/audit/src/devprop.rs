//! Декодування сирих байтів властивостей пристрою (`SetupDiGetDevicePropertyW`).

/// `DEVPROP_TYPE_UINT32` → u32.
pub fn decode_u32(buf: &[u8]) -> Option<u32> {
    let b: [u8; 4] = buf.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(b))
}

/// `DEVPROP_TYPE_FILETIME` → u64.
pub fn decode_filetime(buf: &[u8]) -> Option<u64> {
    let b: [u8; 8] = buf.get(..8)?.try_into().ok()?;
    Some(u64::from_le_bytes(b))
}

fn utf16(buf: &[u8]) -> Vec<u16> {
    buf.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// `DEVPROP_TYPE_STRING` → рядок до першого нуля.
pub fn decode_string(buf: &[u8]) -> String {
    let units = utf16(buf);
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// `DEVPROP_TYPE_STRING_LIST` → список рядків.
pub fn decode_string_list(buf: &[u8]) -> Vec<String> {
    utf16(buf)
        .split(|&u| u == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// UTF-16 з фіксованого масиву WinAPI (до першого нуля).
pub fn wide_array_to_string(units: &[u16]) -> String {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(parts: &[&str]) -> Vec<u8> {
        parts
            .iter()
            .flat_map(|p| p.encode_utf16().chain(std::iter::once(0)))
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    #[test]
    fn decodes_values() {
        assert_eq!(decode_u32(&16u32.to_le_bytes()), Some(16));
        assert_eq!(decode_u32(&[1]), None);
        assert_eq!(decode_filetime(&7u64.to_le_bytes()), Some(7));
        assert_eq!(decode_string(&wide(&["NVIDIA GeForce"])), "NVIDIA GeForce");
        assert_eq!(
            decode_string_list(&wide(&[r"PCI\VEN_10DE&DEV_2783", r"PCI\VEN_10DE"])),
            vec![
                r"PCI\VEN_10DE&DEV_2783".to_string(),
                r"PCI\VEN_10DE".to_string()
            ]
        );
        assert_eq!(wide_array_to_string(&[0x41, 0x42, 0, 0x43]), "AB");
    }
}
