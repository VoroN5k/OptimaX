//! Дані, які збирає проба системи. Правила аудиту працюють лише з ними.

use optimax_core::CivilDate;
use serde::{Deserialize, Serialize};

/// Тип модуля пам'яті (SMBIOS Type 17, поле Memory Type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// DDR3
    Ddr3,
    /// DDR4
    Ddr4,
    /// DDR5
    Ddr5,
    /// LPDDR4 (розпаяна)
    Lpddr4,
    /// LPDDR5 (розпаяна)
    Lpddr5,
    /// Інший код SMBIOS.
    Other(u8),
}

impl MemoryType {
    /// Тип із коду SMBIOS.
    pub fn from_smbios(code: u8) -> Self {
        match code {
            0x18 => Self::Ddr3,
            0x1A => Self::Ddr4,
            0x1E => Self::Lpddr4,
            0x22 => Self::Ddr5,
            0x23 => Self::Lpddr5,
            other => Self::Other(other),
        }
    }

    /// Назва для показу.
    pub fn label(self) -> String {
        match self {
            Self::Ddr3 => "DDR3".into(),
            Self::Ddr4 => "DDR4".into(),
            Self::Ddr5 => "DDR5".into(),
            Self::Lpddr4 => "LPDDR4".into(),
            Self::Lpddr5 => "LPDDR5".into(),
            Self::Other(c) => format!("0x{c:02X}"),
        }
    }
}

/// Встановлений модуль пам'яті.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemoryModule {
    /// Слот (Device Locator), напр. `DIMM_A2`.
    pub locator: String,
    /// Обсяг, МіБ.
    pub size_mib: u64,
    /// Тип пам'яті.
    pub memory_type: MemoryType,
    /// Максимальна швидкість, яку повідомляє модуль (SPD), MT/s. Зазвичай це JEDEC, а не XMP.
    pub rated_mts: Option<u32>,
    /// Поточна налаштована швидкість, MT/s.
    pub configured_mts: Option<u32>,
    /// Виробник.
    pub manufacturer: String,
    /// Партномер.
    pub part_number: String,
}

/// Відомості про BIOS/UEFI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BiosInfo {
    /// Виробник.
    pub vendor: String,
    /// Версія.
    pub version: String,
    /// Дата випуску.
    pub release_date: Option<CivilDate>,
}

/// Дані з таблиці SMBIOS.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FirmwareInfo {
    /// BIOS (Type 0).
    pub bios: Option<BiosInfo>,
    /// Встановлені модулі (Type 17, лише зайняті слоти).
    pub memory: Vec<MemoryModule>,
}

/// Активний монітор.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplayInfo {
    /// Системне ім'я виходу, напр. `\\.\DISPLAY1`.
    pub device_name: String,
    /// Назва монітора.
    pub monitor_name: String,
    /// Головний монітор.
    pub primary: bool,
    /// Поточна ширина, пікс.
    pub width: u32,
    /// Поточна висота, пікс.
    pub height: u32,
    /// Поточна частота, Гц.
    pub current_hz: u32,
    /// Найвища частота, яку Windows пропонує для поточної роздільності, Гц.
    pub max_hz_at_current_res: u32,
}

/// Виробник GPU за PCI Vendor ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    /// 0x10DE
    Nvidia,
    /// 0x1002
    Amd,
    /// 0x8086
    Intel,
    /// Інший.
    Other(u16),
}

impl GpuVendor {
    /// Виробник із PCI Vendor ID.
    pub fn from_pci_id(id: u16) -> Self {
        match id {
            0x10DE => Self::Nvidia,
            0x1002 => Self::Amd,
            0x8086 => Self::Intel,
            other => Self::Other(other),
        }
    }
}

/// Стан з'єднання PCI Express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PcieLink {
    /// Поточна кількість ліній.
    pub current_width: u32,
    /// Максимальна кількість ліній, яку підтримує пристрій.
    pub max_width: u32,
    /// Поточне покоління (у простої часто нижче за максимум — це норма).
    pub current_gen: Option<u32>,
    /// Максимальне покоління пристрою.
    pub max_gen: Option<u32>,
}

/// Стан Resizable BAR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum RebarState {
    /// Вікно BAR1 не менше за VRAM.
    Enabled {
        /// Розмір BAR1, МіБ.
        bar1_mib: u64,
    },
    /// Вікно BAR1 маленьке (типово 256 МіБ).
    Disabled {
        /// Розмір BAR1, МіБ.
        bar1_mib: u64,
    },
    /// Визначити не вдалося.
    Unknown,
}

/// Відеоадаптер.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GpuAdapter {
    /// Назва.
    pub name: String,
    /// Виробник.
    pub vendor: GpuVendor,
    /// Версія драйвера у форматі Windows, напр. `32.0.16.1062`.
    pub driver_version: Option<String>,
    /// Дата драйвера.
    pub driver_date: Option<CivilDate>,
    /// З'єднання PCIe (немає для більшості вбудованих GPU).
    pub pcie: Option<PcieLink>,
    /// Resizable BAR.
    pub rebar: RebarState,
    /// Обсяг VRAM, МіБ (якщо відомий).
    pub vram_mib: Option<u64>,
}

/// Активна схема живлення.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PowerPlan {
    /// GUID у нижньому регістрі, `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    pub guid: String,
    /// Назва схеми (локалізована Windows).
    pub name: Option<String>,
}

/// Ігровий режим Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GameModeState {
    /// Явно ввімкнено.
    Enabled,
    /// Явно вимкнено.
    Disabled,
    /// Значення не задане — Windows 10 1903+ і 11 вважають його ввімкненим.
    Default,
}

/// Апаратне планування GPU (HAGS), значення `HwSchMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HagsState {
    /// `HwSchMode = 2`
    Enabled,
    /// `HwSchMode = 1`
    Disabled,
    /// Значення немає — діє типове значення ОС/драйвера.
    NotSet,
    /// Неочікуване значення.
    Unexpected(u32),
}

/// Реальний стан HAGS одного адаптера за даними драйвера (`D3DKMT_WDDM_2_7_CAPS`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HagsAdapter {
    /// Назва адаптера.
    pub name: String,
    /// Драйвер підтримує HAGS.
    pub supported: bool,
    /// HAGS діє зараз (у поточному сеансі, після останнього перезавантаження).
    pub enabled: bool,
    /// Драйвер вмикає HAGS за замовчуванням.
    pub enabled_by_default: bool,
}

/// Повна картина HAGS: налаштування в реєстрі + що реально діє.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HagsInfo {
    /// Значення `HwSchMode` у реєстрі.
    pub registry: HagsState,
    /// Стан за адаптерами; `None`, якщо запит до драйвера недоступний.
    pub adapters: Option<Vec<HagsAdapter>>,
}

pub use optimax_core::power::format_guid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_format() {
        assert_eq!(
            format_guid(0x381b4222_f694_41f0_9685_ff5bb260df2e),
            "381b4222-f694-41f0-9685-ff5bb260df2e"
        );
    }

    #[test]
    fn memory_type_codes() {
        assert_eq!(MemoryType::from_smbios(0x1A), MemoryType::Ddr4);
        assert_eq!(MemoryType::from_smbios(0x22), MemoryType::Ddr5);
        assert_eq!(MemoryType::from_smbios(0x02).label(), "0x02");
    }
}
