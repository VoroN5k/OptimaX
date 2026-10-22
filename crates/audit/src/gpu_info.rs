//! Чисті допоміжні функції для відомостей про відеоадаптер.

use crate::model::{GpuVendor, RebarState};

/// Витягує PCI Vendor ID з Hardware ID на кшталт `PCI\VEN_10DE&DEV_2783&SUBSYS_...`.
pub fn pci_vendor_from_hardware_id(id: &str) -> Option<u16> {
    let upper = id.to_ascii_uppercase();
    if !upper.starts_with("PCI\\") {
        return None;
    }
    let pos = upper.find("VEN_")?;
    let hex = upper.get(pos + 4..pos + 8)?;
    u16::from_str_radix(hex, 16).ok()
}

/// Чи це віртуальний/програмний адаптер Microsoft (Basic Display, Remote Display).
pub fn is_microsoft_virtual(vendor: GpuVendor) -> bool {
    vendor == GpuVendor::Other(0x1414)
}

/// Переводить версію драйвера Windows у версію NVIDIA: `32.0.16.1062` → `610.62`.
///
/// Правило: останні 5 цифр з'єднання третьої та четвертої груп (четверта доповнена до 4 цифр).
pub fn nvidia_version(windows_version: &str) -> Option<String> {
    let parts: Vec<&str> = windows_version.trim().split('.').collect();
    if parts.len() != 4
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let fourth: u32 = parts[3].parse().ok()?;
    if fourth > 9999 {
        return None;
    }
    let joined = format!("{}{fourth:04}", parts[2]);
    let digits = joined.get(joined.len().checked_sub(5)?..)?;
    let major: u32 = digits[..3].parse().ok()?;
    Some(format!("{major}.{}", &digits[3..]))
}

/// Визначає стан Resizable BAR за розміром вікна BAR1 і обсягом VRAM (метод для NVIDIA).
///
/// Без ReBAR BAR1 зазвичай 256 МіБ; з ReBAR — не менше за обсяг VRAM.
pub fn rebar_from_bar1(bar1_mib: u64, vram_mib: u64) -> RebarState {
    if vram_mib == 0 {
        return RebarState::Unknown;
    }
    if bar1_mib.saturating_mul(10) >= vram_mib.saturating_mul(9) {
        RebarState::Enabled { bar1_mib }
    } else if bar1_mib <= 512 {
        RebarState::Disabled { bar1_mib }
    } else {
        RebarState::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_from_hardware_id() {
        assert_eq!(
            pci_vendor_from_hardware_id(r"PCI\VEN_10DE&DEV_2783&SUBSYS_51101458"),
            Some(0x10DE)
        );
        assert_eq!(
            pci_vendor_from_hardware_id(r"pci\ven_1002&dev_744c"),
            Some(0x1002)
        );
        assert_eq!(pci_vendor_from_hardware_id(r"ROOT\BasicDisplay"), None);
        assert_eq!(pci_vendor_from_hardware_id(r"PCI\VEN_ZZ"), None);
    }

    #[test]
    fn nvidia_driver_versions() {
        assert_eq!(nvidia_version("32.0.16.1062").as_deref(), Some("610.62"));
        assert_eq!(nvidia_version("32.0.15.6094").as_deref(), Some("560.94"));
        assert_eq!(nvidia_version("31.0.15.5222").as_deref(), Some("552.22"));
        assert_eq!(nvidia_version("27.21.14.5671").as_deref(), Some("456.71"));
        assert_eq!(nvidia_version("32.0.16.62").as_deref(), Some("600.62"));
        assert_eq!(nvidia_version("1.2.3"), None);
        assert_eq!(nvidia_version("a.b.c.d"), None);
    }

    #[test]
    fn rebar_detection() {
        assert_eq!(
            rebar_from_bar1(16384, 12282),
            RebarState::Enabled { bar1_mib: 16384 }
        );
        assert_eq!(
            rebar_from_bar1(256, 12282),
            RebarState::Disabled { bar1_mib: 256 }
        );
        assert_eq!(rebar_from_bar1(4096, 12282), RebarState::Unknown);
        assert_eq!(rebar_from_bar1(256, 0), RebarState::Unknown);
    }
}
