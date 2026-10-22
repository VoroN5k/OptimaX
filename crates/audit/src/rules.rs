//! Правила аудиту. Кожне — чиста функція «дані → знахідки», без системних викликів.

use crate::gpu_info::nvidia_version;
use crate::model::{
    BiosInfo, DisplayInfo, GameModeState, GpuAdapter, GpuVendor, HagsAdapter, HagsInfo, HagsState,
    MemoryModule, MemoryType, PowerPlan, RebarState,
};
use optimax_core::{Category, CivilDate, Evidence, Finding, FindingBuilder, Impact, Text};

/// Допуск: пам'ять вважається нижче номіналу, якщо працює повільніше на ≥ 3 %.
const RAM_BELOW_RATED_RATIO: f64 = 0.97;
/// Найвища «базова» частота JEDEC, з якою зазвичай стартують ігрові DDR4-набори без XMP.
const DDR4_JEDEC_BASE_MAX: u32 = 2666;
/// Те саме для DDR5 без EXPO/XMP.
const DDR5_JEDEC_BASE_MAX: u32 = 4800;
/// Мінімальний запас частоти монітора, щоб вважати його невикористаним (Гц і частка).
const DISPLAY_MIN_GAIN_HZ: u32 = 10;
const DISPLAY_MIN_GAIN_RATIO: f64 = 1.15;
/// Вік драйвера (днів), після якого радимо оновитись / просто повідомляємо.
const DRIVER_OLD_DAYS: i64 = 365;
const DRIVER_AGING_DAYS: i64 = 180;
/// Вік BIOS (днів), після якого варто перевірити оновлення.
const BIOS_OLD_DAYS: i64 = 730;

/// GUID стандартних схем живлення Windows.
pub use optimax_core::power::guids as power_guids;

/// Будівник знахідки аудиту: ключі тексту `audit.<id>.title` / `.detail` / `.advice`.
fn draft(id: &'static str, category: Category, impact: Impact, confidence: u8) -> FindingBuilder {
    FindingBuilder::new("audit", id, category, impact, confidence)
}

fn or_unknown<T: ToString>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "?".into())
}

/// Пам'ять: робота нижче номіналу модуля або на базовій частоті JEDEC.
pub fn check_memory(modules: &[MemoryModule]) -> Vec<Finding> {
    let Some(first) = modules.first() else {
        return Vec::new();
    };
    let evidence: Vec<Evidence> = modules
        .iter()
        .map(|m| {
            Evidence::new(
                Text::new("audit.ev.module").arg("slot", &m.locator),
                Text::new("audit.ev.module_value")
                    .arg("size_gb", m.size_mib / 1024)
                    .arg("type", m.memory_type.label())
                    .arg("configured", or_unknown(m.configured_mts))
                    .arg("rated", or_unknown(m.rated_mts))
                    .arg("vendor", &m.manufacturer)
                    .arg("part", &m.part_number),
            )
        })
        .collect();

    let configured = modules.iter().filter_map(|m| m.configured_mts).min();
    let rated = modules.iter().filter_map(|m| m.rated_mts).min();
    let mem_type = first.memory_type;
    let part = modules
        .iter()
        .map(|m| m.part_number.as_str())
        .find(|p| !p.is_empty())
        .unwrap_or("—");

    let Some(configured) = configured else {
        return vec![
            draft("ram.unknown_speed", Category::Memory, Impact::Info, 100)
                .evidence_list(evidence)
                .build(),
        ];
    };

    let draft = match rated {
        Some(rated) if f64::from(configured) < f64::from(rated) * RAM_BELOW_RATED_RATIO => {
            draft("ram.below_rated", Category::Memory, Impact::High, 85)
                .arg("configured", configured)
                .arg("rated", rated)
                .advice()
        }
        _ => {
            let jedec_base = match mem_type {
                MemoryType::Ddr4 => configured <= DDR4_JEDEC_BASE_MAX,
                MemoryType::Ddr5 => configured <= DDR5_JEDEC_BASE_MAX,
                _ => false,
            };
            if jedec_base {
                draft("ram.jedec_base", Category::Memory, Impact::Moderate, 55)
                    .arg("configured", configured)
                    .arg("type", mem_type.label())
                    .arg("part", part)
                    .advice()
            } else {
                draft("ram.ok", Category::Memory, Impact::Info, 80).arg("configured", configured)
            }
        }
    };
    vec![draft.evidence_list(evidence).build()]
}

/// Монітори: частота нижча за доступну для поточної роздільності.
pub fn check_displays(displays: &[DisplayInfo]) -> Vec<Finding> {
    displays
        .iter()
        .map(|d| {
            let gain = d.max_hz_at_current_res.saturating_sub(d.current_hz);
            let underused = gain >= DISPLAY_MIN_GAIN_HZ
                && f64::from(d.max_hz_at_current_res)
                    >= f64::from(d.current_hz) * DISPLAY_MIN_GAIN_RATIO;
            let draft = if underused {
                draft("display.low_refresh", Category::Display, Impact::High, 95).advice()
            } else {
                draft("display.ok", Category::Display, Impact::Info, 90)
            };
            draft
                .arg("monitor", &d.monitor_name)
                .arg("current", d.current_hz)
                .arg("max", d.max_hz_at_current_res)
                .arg("width", d.width)
                .arg("height", d.height)
                .evidence(Text::new("audit.ev.output"), Text::raw(&d.device_name))
                .evidence(
                    Text::new("audit.ev.mode"),
                    Text::new("audit.ev.mode_value")
                        .arg("width", d.width)
                        .arg("height", d.height)
                        .arg("hz", d.current_hz),
                )
                .build()
        })
        .collect()
}

/// PCIe: менше ліній, ніж підтримує карта. Покоління не оцінюємо — у простої воно падає штатно.
pub fn check_pcie(gpu: &GpuAdapter) -> Option<Finding> {
    let link = gpu.pcie?;
    if link.current_width == 0 || link.max_width == 0 {
        return None;
    }
    let draft = if link.current_width < link.max_width {
        let impact = if link.current_width <= 4 {
            Impact::High
        } else {
            Impact::Moderate
        };
        draft("gpu.pcie_narrow", Category::Gpu, impact, 75).advice()
    } else {
        draft("gpu.pcie_ok", Category::Gpu, Impact::Info, 85)
    };
    Some(
        draft
            .arg("gpu", &gpu.name)
            .arg("current", link.current_width)
            .arg("max", link.max_width)
            .evidence(
                Text::new("audit.ev.pcie_width"),
                Text::new("audit.ev.of")
                    .arg("cur", format!("x{}", link.current_width))
                    .arg("max", format!("x{}", link.max_width)),
            )
            .evidence(
                Text::new("audit.ev.pcie_gen"),
                Text::new("audit.ev.of")
                    .arg("cur", or_unknown(link.current_gen))
                    .arg("max", or_unknown(link.max_gen)),
            )
            .build(),
    )
}

/// Resizable BAR. Невідомий стан показуємо лише для дискретних карт (є дані PCIe).
pub fn check_rebar(gpu: &GpuAdapter) -> Option<Finding> {
    let vram = or_unknown(gpu.vram_mib);
    let draft = match gpu.rebar {
        RebarState::Enabled { bar1_mib } => {
            draft("gpu.rebar_on", Category::Gpu, Impact::Info, 80).arg("bar1", bar1_mib)
        }
        RebarState::Disabled { bar1_mib } => {
            draft("gpu.rebar_off", Category::Gpu, Impact::Moderate, 70)
                .arg("bar1", bar1_mib)
                .advice()
        }
        RebarState::Unknown if gpu.pcie.is_some() => {
            draft("gpu.rebar_unknown", Category::Gpu, Impact::Info, 100)
        }
        RebarState::Unknown => return None,
    };
    Some(draft.arg("gpu", &gpu.name).arg("vram", vram).build())
}

/// Вік драйвера відеокарти.
pub fn check_driver(gpu: &GpuAdapter, today: CivilDate) -> Option<Finding> {
    let date = gpu.driver_date?;
    let raw_version = gpu.driver_version.clone().unwrap_or_else(|| "?".into());
    let version = match (gpu.vendor, nvidia_version(&raw_version)) {
        (GpuVendor::Nvidia, Some(nv)) => format!("{nv} ({raw_version})"),
        _ => raw_version,
    };
    let days = date.days_until(today).max(0);
    let draft = if days > DRIVER_OLD_DAYS {
        draft("driver.old", Category::Driver, Impact::Moderate, 60).advice()
    } else if days > DRIVER_AGING_DAYS {
        draft("driver.aging", Category::Driver, Impact::Info, 60).advice()
    } else {
        draft("driver.ok", Category::Driver, Impact::Info, 90)
    };
    Some(
        draft
            .arg("gpu", &gpu.name)
            .arg("version", version)
            .arg("date", date)
            .arg("days", days)
            .build(),
    )
}

/// BIOS: лише інформація; старий BIOS — привід перевірити оновлення, а не обіцянка FPS.
pub fn check_bios(bios: &BiosInfo, today: CivilDate) -> Finding {
    let date = bios
        .release_date
        .map(|d| d.to_string())
        .unwrap_or_else(|| "?".into());
    let old = bios
        .release_date
        .is_some_and(|d| d.days_until(today) > BIOS_OLD_DAYS);
    let draft = if old {
        draft("bios.old", Category::Firmware, Impact::Info, 70)
            .arg("years", BIOS_OLD_DAYS / 365)
            .advice()
    } else {
        draft("bios.info", Category::Firmware, Impact::Info, 95)
    };
    draft
        .arg("vendor", &bios.vendor)
        .arg("version", &bios.version)
        .arg("date", date)
        .build()
}

/// Активна схема живлення.
pub fn check_power_plan(plan: &PowerPlan) -> Finding {
    let name = plan.name.clone().unwrap_or_else(|| plan.guid.clone());
    let draft = match plan.guid.as_str() {
        power_guids::POWER_SAVER => {
            draft("power.saver", Category::Power, Impact::High, 85).advice()
        }
        power_guids::BALANCED => draft("power.balanced", Category::Power, Impact::Info, 80),
        power_guids::HIGH_PERFORMANCE | power_guids::ULTIMATE => {
            draft("power.high", Category::Power, Impact::Info, 90)
        }
        _ => draft("power.custom", Category::Power, Impact::Info, 70),
    };
    draft
        .arg("name", name)
        .evidence(Text::new("audit.ev.guid"), Text::raw(&plan.guid))
        .build()
}

/// Ігровий режим Windows. Вплив зазвичай малий — лише інформуємо.
pub fn check_game_mode(state: GameModeState) -> Finding {
    match state {
        GameModeState::Enabled => draft("os.game_mode_on", Category::Os, Impact::Info, 90),
        GameModeState::Default => draft("os.game_mode_default", Category::Os, Impact::Info, 80),
        GameModeState::Disabled => {
            draft("os.game_mode_off", Category::Os, Impact::Info, 90).advice()
        }
    }
    .build()
}

/// Апаратне планування GPU (HAGS). Реальний стан від драйвера має пріоритет; реєстр — запасне
/// джерело й ознака того, що зміна ще чекає перезавантаження.
pub fn check_hags(info: &HagsInfo) -> Finding {
    let registry = match info.registry {
        HagsState::Enabled => Text::new("audit.ev.hags_reg_on"),
        HagsState::Disabled => Text::new("audit.ev.hags_reg_off"),
        HagsState::NotSet => Text::new("audit.ev.hags_reg_unset"),
        HagsState::Unexpected(v) => Text::new("audit.ev.hags_reg_unexpected").arg("value", v),
    };

    let Some(adapters) = &info.adapters else {
        let draft = match info.registry {
            HagsState::Enabled => draft("os.hags_on", Category::Os, Impact::Info, 90),
            HagsState::Disabled => draft("os.hags_off", Category::Os, Impact::Info, 90).advice(),
            HagsState::NotSet => draft("os.hags_default", Category::Os, Impact::Info, 60),
            HagsState::Unexpected(v) => {
                draft("os.hags_unexpected", Category::Os, Impact::Info, 50).arg("value", v)
            }
        };
        return draft
            .evidence(Text::new("audit.ev.hags_registry"), registry)
            .build();
    };

    let supported: Vec<&HagsAdapter> = adapters.iter().filter(|a| a.supported).collect();
    let active = supported.iter().any(|a| a.enabled);
    let mut draft = if supported.is_empty() {
        draft("os.hags_unsupported", Category::Os, Impact::Info, 90)
    } else {
        match (active, info.registry) {
            (false, HagsState::Enabled) => {
                draft("os.hags_pending_on", Category::Os, Impact::Info, 90)
            }
            (true, HagsState::Disabled) => {
                draft("os.hags_pending_off", Category::Os, Impact::Info, 90)
            }
            (true, _) => draft("os.hags_on", Category::Os, Impact::Info, 95),
            (false, _) => draft("os.hags_off", Category::Os, Impact::Info, 95).advice(),
        }
    }
    .evidence(Text::new("audit.ev.hags_registry"), registry);

    // Показуємо адаптери з підтримкою HAGS; якщо таких немає — усі, щоб було видно чому.
    let shown: Vec<&HagsAdapter> = if supported.is_empty() {
        adapters.iter().collect()
    } else {
        supported
    };
    for a in shown {
        let state = match (a.supported, a.enabled) {
            (true, true) => "audit.ev.hags_active",
            (true, false) => "audit.ev.hags_inactive",
            (false, _) => "audit.ev.hags_no_support",
        };
        draft = draft.evidence(
            Text::new("audit.ev.adapter").arg("name", &a.name),
            Text::new(state),
        );
    }
    draft.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PcieLink;

    fn module(t: MemoryType, rated: Option<u32>, configured: Option<u32>) -> MemoryModule {
        MemoryModule {
            locator: "DIMM_A2".into(),
            size_mib: 16384,
            memory_type: t,
            rated_mts: rated,
            configured_mts: configured,
            manufacturer: "Kingston".into(),
            part_number: "KF432C16BB/16".into(),
        }
    }

    fn gpu() -> GpuAdapter {
        GpuAdapter {
            name: "NVIDIA GeForce RTX 4070 SUPER".into(),
            vendor: GpuVendor::Nvidia,
            driver_version: Some("32.0.16.1062".into()),
            driver_date: CivilDate::new(2026, 9, 1),
            pcie: Some(PcieLink {
                current_width: 16,
                max_width: 16,
                current_gen: Some(1),
                max_gen: Some(4),
            }),
            rebar: RebarState::Enabled { bar1_mib: 16384 },
            vram_mib: Some(12282),
        }
    }

    fn today() -> CivilDate {
        CivilDate::new(2026, 10, 22).unwrap()
    }

    #[test]
    fn ram_below_rated_is_high() {
        let f = check_memory(&[module(MemoryType::Ddr4, Some(3200), Some(2133))]);
        assert_eq!(f[0].id, "ram.below_rated");
        assert_eq!(f[0].impact, Impact::High);
        assert_eq!(f[0].title.args["configured"], "2133");
        assert!(f[0].advice.is_some());
    }

    #[test]
    fn ram_jedec_base_is_moderate_with_lower_confidence() {
        let f = check_memory(&[module(MemoryType::Ddr4, Some(2133), Some(2133))]);
        assert_eq!(f[0].id, "ram.jedec_base");
        assert_eq!(f[0].impact, Impact::Moderate);
        assert!(f[0].confidence.percent() < 70);
        let f = check_memory(&[module(MemoryType::Ddr5, Some(4800), Some(4800))]);
        assert_eq!(f[0].id, "ram.jedec_base");
    }

    #[test]
    fn ram_ok_cases() {
        assert_eq!(
            check_memory(&[module(MemoryType::Ddr4, Some(3600), Some(3600))])[0].id,
            "ram.ok"
        );
        assert_eq!(
            check_memory(&[module(MemoryType::Ddr5, Some(6000), Some(6000))])[0].id,
            "ram.ok"
        );
        // Розпаяна LPDDR на «базовій» частоті — XMP недоступний, не лякаємо.
        assert_eq!(
            check_memory(&[module(MemoryType::Lpddr5, Some(4800), Some(4800))])[0].id,
            "ram.ok"
        );
        // Різниця в межах допуску.
        assert_eq!(
            check_memory(&[module(MemoryType::Ddr4, Some(3600), Some(3533))])[0].id,
            "ram.ok"
        );
    }

    #[test]
    fn ram_uses_slowest_module_and_handles_unknown() {
        let f = check_memory(&[
            module(MemoryType::Ddr4, Some(3200), Some(3200)),
            module(MemoryType::Ddr4, Some(3200), Some(2400)),
        ]);
        assert_eq!(f[0].id, "ram.below_rated");
        assert_eq!(f[0].evidence.len(), 2);
        assert_eq!(
            check_memory(&[module(MemoryType::Ddr4, None, None)])[0].id,
            "ram.unknown_speed"
        );
        assert!(check_memory(&[]).is_empty());
    }

    fn display(current: u32, max: u32) -> DisplayInfo {
        DisplayInfo {
            device_name: r"\\.\DISPLAY1".into(),
            monitor_name: "Generic PnP Monitor".into(),
            primary: true,
            width: 2560,
            height: 1440,
            current_hz: current,
            max_hz_at_current_res: max,
        }
    }

    #[test]
    fn display_refresh_rules() {
        let f = check_displays(&[
            display(60, 165),
            display(165, 165),
            display(59, 60),
            display(144, 155),
        ]);
        let ids: Vec<&str> = f.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "display.low_refresh",
                "display.ok",
                "display.ok",
                "display.ok"
            ]
        );
        assert_eq!(f[0].impact, Impact::High);
        assert_eq!(f[0].title.args["max"], "165");
    }

    #[test]
    fn pcie_rules() {
        let mut g = gpu();
        assert_eq!(check_pcie(&g).unwrap().id, "gpu.pcie_ok");
        g.pcie = Some(PcieLink {
            current_width: 8,
            max_width: 16,
            current_gen: Some(4),
            max_gen: Some(4),
        });
        let f = check_pcie(&g).unwrap();
        assert_eq!(
            (f.id.as_str(), f.impact),
            ("gpu.pcie_narrow", Impact::Moderate)
        );
        g.pcie = Some(PcieLink {
            current_width: 4,
            max_width: 16,
            current_gen: None,
            max_gen: None,
        });
        assert_eq!(check_pcie(&g).unwrap().impact, Impact::High);
        // Карта з рідними x8 (напр. RTX 4060) — не проблема.
        g.pcie = Some(PcieLink {
            current_width: 8,
            max_width: 8,
            current_gen: Some(4),
            max_gen: Some(4),
        });
        assert_eq!(check_pcie(&g).unwrap().id, "gpu.pcie_ok");
        g.pcie = None;
        assert!(check_pcie(&g).is_none());
    }

    #[test]
    fn rebar_rules() {
        let mut g = gpu();
        assert_eq!(check_rebar(&g).unwrap().id, "gpu.rebar_on");
        g.rebar = RebarState::Disabled { bar1_mib: 256 };
        assert_eq!(check_rebar(&g).unwrap().impact, Impact::Moderate);
        g.rebar = RebarState::Unknown;
        assert_eq!(check_rebar(&g).unwrap().id, "gpu.rebar_unknown");
        g.pcie = None;
        assert!(check_rebar(&g).is_none());
    }

    #[test]
    fn driver_age_rules() {
        let mut g = gpu();
        let f = check_driver(&g, today()).unwrap();
        assert_eq!(f.id, "driver.ok");
        assert_eq!(f.title.args["version"], "610.62 (32.0.16.1062)");
        g.driver_date = CivilDate::new(2026, 3, 1);
        assert_eq!(check_driver(&g, today()).unwrap().id, "driver.aging");
        g.driver_date = CivilDate::new(2025, 1, 1);
        let f = check_driver(&g, today()).unwrap();
        assert_eq!((f.id.as_str(), f.impact), ("driver.old", Impact::Moderate));
        g.driver_date = None;
        assert!(check_driver(&g, today()).is_none());
    }

    #[test]
    fn bios_rules() {
        let mut b = BiosInfo {
            vendor: "AMI".into(),
            version: "F12".into(),
            release_date: CivilDate::new(2025, 6, 1),
        };
        assert_eq!(check_bios(&b, today()).id, "bios.info");
        b.release_date = CivilDate::new(2021, 6, 1);
        let f = check_bios(&b, today());
        assert_eq!((f.id.as_str(), f.impact), ("bios.old", Impact::Info));
        b.release_date = None;
        assert_eq!(check_bios(&b, today()).title.args["date"], "?");
    }

    #[test]
    fn power_plan_rules() {
        let plan = |guid: &str| PowerPlan {
            guid: guid.into(),
            name: Some("X".into()),
        };
        assert_eq!(
            check_power_plan(&plan(power_guids::POWER_SAVER)).impact,
            Impact::High
        );
        assert_eq!(
            check_power_plan(&plan(power_guids::BALANCED)).id,
            "power.balanced"
        );
        assert_eq!(
            check_power_plan(&plan(power_guids::ULTIMATE)).id,
            "power.high"
        );
        assert_eq!(
            check_power_plan(&plan("00000000-0000-0000-0000-000000000001")).id,
            "power.custom"
        );
    }

    #[test]
    fn os_toggles_are_informational() {
        for s in [
            GameModeState::Enabled,
            GameModeState::Disabled,
            GameModeState::Default,
        ] {
            assert_eq!(check_game_mode(s).impact, Impact::Info);
        }
    }

    fn hags(registry: HagsState, adapters: Option<Vec<HagsAdapter>>) -> Finding {
        check_hags(&HagsInfo { registry, adapters })
    }

    fn adapter(name: &str, supported: bool, enabled: bool) -> HagsAdapter {
        HagsAdapter {
            name: name.into(),
            supported,
            enabled,
            enabled_by_default: false,
        }
    }

    #[test]
    fn hags_registry_fallback() {
        let ids: Vec<String> = [
            HagsState::Enabled,
            HagsState::Disabled,
            HagsState::NotSet,
            HagsState::Unexpected(7),
        ]
        .into_iter()
        .map(|s| hags(s, None).id)
        .collect();
        assert_eq!(
            ids,
            vec![
                "os.hags_on",
                "os.hags_off",
                "os.hags_default",
                "os.hags_unexpected"
            ]
        );
        assert!(hags(HagsState::Disabled, None).advice.is_some());
    }

    #[test]
    fn hags_runtime_state_wins_over_unset_registry() {
        let basic = adapter("Microsoft Basic Render Driver", false, false);
        let nv = |enabled| adapter("NVIDIA GeForce RTX 4070 SUPER", true, enabled);

        let f = hags(HagsState::NotSet, Some(vec![basic.clone(), nv(true)]));
        assert_eq!(f.id, "os.hags_on");
        assert_eq!(f.impact, Impact::Info);
        // Реєстр + лише адаптер із підтримкою.
        assert_eq!(f.evidence.len(), 2);

        let f = hags(HagsState::NotSet, Some(vec![nv(false)]));
        assert_eq!(f.id, "os.hags_off");
        assert!(f.advice.is_some());

        assert_eq!(
            hags(HagsState::Enabled, Some(vec![nv(false)])).id,
            "os.hags_pending_on"
        );
        assert_eq!(
            hags(HagsState::Disabled, Some(vec![nv(true)])).id,
            "os.hags_pending_off"
        );

        let f = hags(HagsState::NotSet, Some(vec![basic]));
        assert_eq!(f.id, "os.hags_unsupported");
        assert_eq!(f.evidence.len(), 2);
    }
}
