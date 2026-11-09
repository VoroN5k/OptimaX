//! Датчики процесорів AMD Ryzen через драйвер PawnIO (лише читання, лише за згодою користувача).
//!
//! - Температура Tctl і кожного CCD — регістри SMN, як у драйвері Linux `k10temp`
//!   (адреси й формули звірено з його кодом 2026-11-09; перевірено на Ryzen 7 5800X проти HWiNFO).
//! - FCLK / UCLK / MCLK — таблиця живлення SMU (PM table). Її розкладка залежить від версії
//!   таблиці; відомі версії — у [`pm_layout`] (індекси з LibreHardwareMonitor і ryzen_smu, версія
//!   0x380905 перевірена на 5800X проти ZenTimings). Невідома версія — частот немає, не вгадуємо.
//!
//! Модулі PawnIO (`AMDFamily17`, `RyzenSMU`, LGPL-2.1, підписані автором драйвера) вбудовано з
//! офіційного релізу PawnIO.Modules 0.2.11; SHA-256 перевіряється перед кожним завантаженням.
//! З модулів викликаються лише функції читання: `ioctl_read_smn`, `ioctl_resolve_pm_table`,
//! `ioctl_read_pm_table` і `ioctl_update_pm_table` (просить SMU оновити таблицю в пам'яті — так
//! само роблять ZenTimings і LibreHardwareMonitor). Довільні команди SMU й запис регістрів — ніколи.

use optimax_core::registry::{RegHive, RegistryApi};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Модуль `AMDFamily17` (SMN і MSR для Zen 1–5).
pub const AMD_FAMILY17_BIN: &[u8] = include_bytes!("../pawnio/AMDFamily17.bin");
/// SHA-256 модуля `AMDFamily17.bin` (PawnIO.Modules 0.2.11).
pub const AMD_FAMILY17_SHA256: &str =
    "dae74615761b78bdf064dfb3e136252ddcc6fc727d88f14738d0e5800d427a91";
/// Модуль `RyzenSMU` (таблиця живлення SMU).
pub const RYZEN_SMU_BIN: &[u8] = include_bytes!("../pawnio/RyzenSMU.bin");
/// SHA-256 модуля `RyzenSMU.bin` (PawnIO.Modules 0.2.11).
pub const RYZEN_SMU_SHA256: &str =
    "301d9ca397108e09f31bfbd5ac4c9bb4f352a5de68532c32db3ba7ddcde93450";

/// Регістр звітної температури (Tctl) у просторі SMN.
pub const SMN_TCTL: u32 = 0x0005_9800;
/// Біт «показання дійсне» у регістрі CCD.
const CCD_VALID: u32 = 1 << 11;

/// Помилки датчиків.
#[derive(Debug, thiserror::Error)]
pub enum SensorError {
    /// Вбудований модуль не збігся з еталонним хешем (збірку підмінено) — не завантажуємо.
    #[error("модуль PawnIO {0} не збігся з SHA-256 — не завантажено")]
    BadModule(&'static str),
    /// Процесор не підтримується (не AMD Zen або невідома модель).
    #[error("процесор не підтримується: {0}")]
    Unsupported(String),
    /// Драйвер чи модуль повернув помилку.
    #[error("{0}")]
    Driver(String),
}

/// Модуль збігся з хешем.
pub fn module_ok(blob: &[u8], sha256_hex: &str) -> bool {
    let digest = Sha256::digest(blob);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex == sha256_hex
}

/// Сімейство й модель процесора з `HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuId {
    /// Виробник AMD.
    pub amd: bool,
    /// Сімейство (25 = 19h, Zen 3/4).
    pub family: u32,
    /// Модель.
    pub model: u32,
    /// Назва (`ProcessorNameString`, без хвостових пробілів).
    pub name: String,
}

const CPU_KEY: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

/// Читає [`CpuId`] з реєстру (`None` — немає або формат незнайомий).
pub fn cpu_id(reg: &dyn RegistryApi) -> Option<CpuId> {
    let s = |name: &str| {
        reg.read_value(RegHive::LocalMachine, CPU_KEY, name)
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_string))
    };
    let (family, model) = parse_identifier(&s("Identifier")?)?;
    let vendor = s("VendorIdentifier").unwrap_or_default();
    Some(CpuId {
        amd: vendor.trim() == "AuthenticAMD",
        family,
        model,
        name: s("ProcessorNameString")
            .unwrap_or_default()
            .trim()
            .to_string(),
    })
}

/// `"AMD64 Family 25 Model 33 Stepping 2"` → (25, 33).
pub fn parse_identifier(id: &str) -> Option<(u32, u32)> {
    let mut it = id.split_whitespace();
    let mut family = None;
    let mut model = None;
    while let Some(w) = it.next() {
        match w {
            "Family" => family = it.next().and_then(|v| v.parse().ok()),
            "Model" => model = it.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    Some((family?, model?))
}

/// Де лежать температури CCD: зміщення від [`SMN_TCTL`] і скільки CCD може бути (k10temp).
pub fn ccd_layout(family: u32, model: u32) -> Option<(u32, u32)> {
    match (family, model) {
        (0x17, 0x01 | 0x08 | 0x11 | 0x18) => Some((0x154, 4)),
        (0x17, 0x31 | 0x47 | 0x60 | 0x68 | 0x71) => Some((0x154, 8)),
        (0x17, 0xA0..=0xAF) => Some((0x300, 8)),
        (0x19, 0x00..=0x01 | 0x08 | 0x21 | 0x50..=0x5F) => Some((0x154, 8)),
        (0x19, 0x40..=0x4F) => Some((0x300, 8)),
        (0x19, 0x60..=0x7F) => Some((0x308, 8)),
        (0x19, 0x10..=0x1F | 0xA0..=0xAF) => Some((0x300, 12)),
        (0x1A, 0x00..=0x1F) => Some((0x1F0, 16)),
        (0x1A, 0x40..=0x4F) => Some((0x308, 8)),
        _ => None,
    }
}

/// На скільки Tctl вищий за реальну температуру, °C (k10temp: лише старші X-моделі й Threadripper).
pub fn tctl_offset_c(family: u32, name: &str) -> f32 {
    if family != 0x17 {
        return 0.0;
    }
    const TABLE: [(&str, f32); 6] = [
        ("AMD Ryzen 5 1600X", 20.0),
        ("AMD Ryzen 7 1700X", 20.0),
        ("AMD Ryzen 7 1800X", 20.0),
        ("AMD Ryzen 7 2700X", 10.0),
        ("AMD Ryzen Threadripper 19", 27.0),
        ("AMD Ryzen Threadripper 29", 27.0),
    ];
    TABLE
        .iter()
        .find(|(id, _)| name.contains(id))
        .map_or(0.0, |(_, o)| *o)
}

/// Tctl з регістра, °C: `(reg >> 21) × 0.125`, мінус 49 °C у розширеному діапазоні
/// (біт 19 або біти 16–17 = 11).
pub fn tctl_c(raw: u32) -> f32 {
    let mut m = f64::from(raw >> 21) * 125.0;
    if raw & (1 << 19) != 0 || raw & 0x3_0000 == 0x3_0000 {
        m -= 49_000.0;
    }
    (m / 1000.0) as f32
}

/// Температура CCD з регістра, °C (`None` — показання недійсне або CCD немає).
pub fn ccd_c(raw: u32) -> Option<f32> {
    (raw & CCD_VALID != 0).then(|| (f64::from(raw & 0x7FF) * 125.0 - 49_000.0) as f32 / 1000.0)
}

/// Де в таблиці SMU (індекси `f32`) лежать частоти.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmLayout {
    /// Infinity Fabric.
    pub fclk: usize,
    /// Контролер пам'яті.
    pub uclk: usize,
    /// Пам'ять.
    pub mclk: usize,
}

/// Розкладка для відомих версій таблиці. Zen 2 0x240903 — ryzen_smu і LibreHardwareMonitor;
/// Zen 3 0x380805 — LibreHardwareMonitor; 0x380905 — перевірено на 5800X проти ZenTimings.
pub fn pm_layout(version: u32) -> Option<PmLayout> {
    match version {
        0x0024_0903 | 0x0038_0805 | 0x0038_0905 => Some(PmLayout {
            fclk: 48,
            uclk: 50,
            mclk: 51,
        }),
        _ => None,
    }
}

/// Таблиця з `i64` відповіді драйвера → `f32` (у кожному `i64` два значення, little-endian).
pub fn pm_floats(qwords: &[i64]) -> Vec<f32> {
    qwords
        .iter()
        .flat_map(|q| {
            let b = q.to_le_bytes();
            [
                f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                f32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            ]
        })
        .collect()
}

/// Частоти шини пам'яті, МГц.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemClocks {
    /// Infinity Fabric (FCLK).
    pub fclk_mhz: f32,
    /// Контролер пам'яті (UCLK).
    pub uclk_mhz: f32,
    /// Пам'ять (MCLK; DDR4-3600 = 1800).
    pub mclk_mhz: f32,
}

/// Частоти з таблиці за розкладкою; `None` — індексу немає або значення неправдоподібне.
pub fn mem_clocks(table: &[f32], l: PmLayout) -> Option<MemClocks> {
    let get = |i: usize| {
        table
            .get(i)
            .copied()
            .filter(|v| v.is_finite() && (100.0..=10_000.0).contains(v))
    };
    Some(MemClocks {
        fclk_mhz: get(l.fclk)?,
        uclk_mhz: get(l.uclk)?,
        mclk_mhz: get(l.mclk)?,
    })
}

/// Одне зчитування датчиків.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AmdReading {
    /// Температура процесора (Tctl мінус зсув моделі), °C.
    pub cpu_c: f32,
    /// Температури CCD, °C (лише дійсні).
    pub ccd_c: Vec<f32>,
    /// Частоти шини пам'яті (`None` — версія таблиці SMU невідома).
    pub clocks: Option<MemClocks>,
    /// Версія таблиці SMU (`None` — модуль SMU не завантажився).
    pub pm_version: Option<u32>,
}

#[cfg(windows)]
pub use self::win::AmdSensors;

#[cfg(windows)]
mod win {
    use super::*;
    use optimax_core::pawnio::{Module, PciLock};

    /// Скільки чекати на спільний м'ютекс PCI, мс.
    const PCI_WAIT_MS: u32 = 500;

    fn driver(e: impl std::fmt::Display) -> SensorError {
        SensorError::Driver(e.to_string())
    }

    /// Відкриті модулі PawnIO для цього процесора.
    pub struct AmdSensors {
        smn: Module,
        smu: Option<(Module, PmLayout, u32)>,
        pm_version: Option<u32>,
        ccd_regs: Vec<u32>,
        offset_c: f32,
    }

    impl std::fmt::Debug for AmdSensors {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("AmdSensors")
                .field("pm_version", &self.pm_version)
                .field("ccds", &self.ccd_regs.len())
                .finish()
        }
    }

    impl AmdSensors {
        /// Перевіряє процесор і хеші, завантажує модулі. Таблиця SMU — необов'язкова: без неї
        /// лишається температура.
        pub fn open(reg: &dyn RegistryApi) -> Result<Self, SensorError> {
            let id = cpu_id(reg).ok_or_else(|| SensorError::Unsupported("невідомий".into()))?;
            if !id.amd || !(0x17..=0x1A).contains(&id.family) {
                return Err(SensorError::Unsupported(id.name));
            }
            if !module_ok(AMD_FAMILY17_BIN, AMD_FAMILY17_SHA256) {
                return Err(SensorError::BadModule("AMDFamily17"));
            }
            let smn = Module::load(AMD_FAMILY17_BIN).map_err(driver)?;
            let mut me = Self {
                smn,
                smu: None,
                pm_version: None,
                ccd_regs: Vec::new(),
                offset_c: tctl_offset_c(id.family, &id.name),
            };
            // Які CCD є: ті, що зараз дають дійсне показання.
            if let Some((offset, max)) = ccd_layout(id.family, id.model) {
                let regs: Vec<u32> = (0..max).map(|i| SMN_TCTL + offset + i * 4).collect();
                let lock = PciLock::acquire(PCI_WAIT_MS).map_err(driver)?;
                for r in regs {
                    if me.read_smn(r).ok().and_then(ccd_c).is_some() {
                        me.ccd_regs.push(r);
                    }
                }
                drop(lock);
            }
            match me.open_smu() {
                Ok(v) => me.pm_version = Some(v),
                Err(e) => tracing::info!("таблиця SMU недоступна: {e}"),
            }
            Ok(me)
        }

        fn open_smu(&mut self) -> Result<u32, SensorError> {
            if !module_ok(RYZEN_SMU_BIN, RYZEN_SMU_SHA256) {
                return Err(SensorError::BadModule("RyzenSMU"));
            }
            let smu = Module::load(RYZEN_SMU_BIN).map_err(driver)?;
            let lock = PciLock::acquire(PCI_WAIT_MS).map_err(driver)?;
            let res = smu
                .execute("ioctl_resolve_pm_table", &[], 2)
                .map_err(driver)?;
            drop(lock);
            let version = res
                .first()
                .and_then(|v| u32::try_from(*v).ok())
                .ok_or_else(|| SensorError::Driver("порожня відповідь SMU".into()))?;
            if let Some(layout) = pm_layout(version) {
                let qwords = u32::try_from(layout.fclk.max(layout.uclk).max(layout.mclk) / 2 + 1)
                    .unwrap_or(64);
                self.smu = Some((smu, layout, qwords));
            } else {
                tracing::info!("таблиця SMU 0x{version:X}: розкладка невідома, частот не буде");
            }
            Ok(version)
        }

        fn read_smn(&self, addr: u32) -> Result<u32, SensorError> {
            let v = self
                .smn
                .execute("ioctl_read_smn", &[i64::from(addr)], 1)
                .map_err(driver)?;
            v.first()
                .map(|x| (*x as u64 & 0xFFFF_FFFF) as u32)
                .ok_or_else(|| SensorError::Driver("порожня відповідь SMN".into()))
        }

        /// Лише Tctl, °C — для частих замірів (без таблиці SMU).
        pub fn read_temp(&self) -> Result<f32, SensorError> {
            let lock = PciLock::acquire(PCI_WAIT_MS).map_err(driver)?;
            let raw = self.read_smn(SMN_TCTL)?;
            drop(lock);
            Ok(tctl_c(raw) - self.offset_c)
        }

        /// Читає температури й частоти (під спільним м'ютексом PCI).
        pub fn read(&self) -> Result<AmdReading, SensorError> {
            let lock = PciLock::acquire(PCI_WAIT_MS).map_err(driver)?;
            let cpu_c = tctl_c(self.read_smn(SMN_TCTL)?) - self.offset_c;
            let ccd_c = self
                .ccd_regs
                .iter()
                .filter_map(|r| self.read_smn(*r).ok().and_then(ccd_c))
                .collect();
            let clocks = self.smu.as_ref().and_then(|(m, layout, qwords)| {
                m.execute("ioctl_update_pm_table", &[], 0).ok()?;
                let q = m
                    .execute("ioctl_read_pm_table", &[], *qwords as usize)
                    .ok()?;
                mem_clocks(&pm_floats(&q), *layout)
            });
            drop(lock);
            Ok(AmdReading {
                cpu_c,
                ccd_c,
                clocks,
                pm_version: self.pm_version,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::registry::{MemRegistry, RegValue};

    #[test]
    fn embedded_modules_match_their_hashes() {
        assert!(module_ok(AMD_FAMILY17_BIN, AMD_FAMILY17_SHA256));
        assert!(module_ok(RYZEN_SMU_BIN, RYZEN_SMU_SHA256));
        assert!(!module_ok(b"x", AMD_FAMILY17_SHA256));
    }

    #[test]
    fn temperatures_match_k10temp_and_the_real_5800x() {
        // 5800X 2026-11-09: 0x609B0000 → 47.5 °C (HWiNFO показав те саме).
        assert!((tctl_c(0x609B_0000) - 47.5).abs() < 0.01);
        // Розширений діапазон (біт 19): мінус 49 °C.
        assert!((tctl_c((800 << 21) | (1 << 19)) - 51.0).abs() < 0.01);
        assert!((tctl_c((800 << 21) | 0x3_0000) - 51.0).abs() < 0.01);
        // CCD: (значення × 0.125) − 49, лише з бітом «дійсне».
        assert!((ccd_c(CCD_VALID | 672).expect("дійсне") - 35.0).abs() < 0.01);
        assert_eq!(ccd_c(672), None);
    }

    #[test]
    fn cpu_identity_and_ccd_layout() {
        assert_eq!(
            parse_identifier("AMD64 Family 25 Model 33 Stepping 2"),
            Some((25, 33))
        );
        assert_eq!(parse_identifier("щось"), None);
        let mut reg = MemRegistry::new();
        reg.create_key(RegHive::LocalMachine, CPU_KEY);
        let put = |r: &mut MemRegistry, n: &str, v: &str| {
            r.set(
                RegHive::LocalMachine,
                CPU_KEY,
                n,
                RegValue::String(v.into()),
            );
        };
        put(
            &mut reg,
            "Identifier",
            "AMD64 Family 25 Model 33 Stepping 2",
        );
        put(&mut reg, "VendorIdentifier", "AuthenticAMD");
        put(
            &mut reg,
            "ProcessorNameString",
            "AMD Ryzen 7 5800X 8-Core Processor   ",
        );
        let id = cpu_id(&reg).expect("ідентифікатор");
        assert!(id.amd);
        assert_eq!((id.family, id.model), (0x19, 0x21));
        assert_eq!(id.name, "AMD Ryzen 7 5800X 8-Core Processor");
        assert_eq!(ccd_layout(0x19, 0x21), Some((0x154, 8)));
        assert_eq!(ccd_layout(0x19, 0x99), None);
        assert_eq!(tctl_offset_c(0x19, &id.name), 0.0);
        assert_eq!(
            tctl_offset_c(0x17, "AMD Ryzen 7 2700X Eight-Core Processor"),
            10.0
        );
    }

    #[test]
    fn memory_clocks_only_for_known_tables() {
        // Перші 52 значення таблиці 5800X (0x380905) з розвідки: [48..=51] = 1800.
        let mut t = vec![0.0f32; 60];
        t[48] = 1800.0;
        t[49] = 1800.02;
        t[50] = 1800.0;
        t[51] = 1800.0;
        let q: Vec<i64> = t
            .chunks(2)
            .map(|p| {
                let mut b = [0u8; 8];
                b[..4].copy_from_slice(&p[0].to_le_bytes());
                b[4..].copy_from_slice(&p[1].to_le_bytes());
                i64::from_le_bytes(b)
            })
            .collect();
        let floats = pm_floats(&q);
        assert_eq!(floats.len(), 60);
        let l = pm_layout(0x0038_0905).expect("відома");
        let c = mem_clocks(&floats, l).expect("частоти");
        assert_eq!(
            (c.fclk_mhz, c.uclk_mhz, c.mclk_mhz),
            (1800.0, 1800.0, 1800.0)
        );
        assert!(pm_layout(0x0054_0004).is_none());
        // Нулі чи сміття — не частоти.
        assert!(mem_clocks(&vec![0.0; 60], l).is_none());
        assert!(mem_clocks(&floats[..40], l).is_none());
    }
}
