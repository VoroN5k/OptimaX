//! Запуск усіх перевірок і звіт.

use crate::probe::SystemProbe;
use crate::rules;
use optimax_core::{CivilDate, CoreError, Finding, Text};
use serde::{Deserialize, Serialize};

/// Група перевірок, що читає одне джерело даних.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    /// SMBIOS: пам'ять і BIOS.
    Firmware,
    /// Монітори.
    Displays,
    /// Відеоадаптери.
    Gpus,
    /// Схема живлення.
    PowerPlan,
    /// Ігровий режим.
    GameMode,
    /// HAGS.
    Hags,
}

impl CheckId {
    /// Локалізована назва перевірки.
    pub fn name(self) -> Text {
        Text::new(match self {
            Self::Firmware => "audit.check.firmware",
            Self::Displays => "audit.check.displays",
            Self::Gpus => "audit.check.gpus",
            Self::PowerPlan => "audit.check.power_plan",
            Self::GameMode => "audit.check.game_mode",
            Self::Hags => "audit.check.hags",
        })
    }
}

/// Перевірка, яку не вдалося виконати. Показується користувачу, а не замовчується.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkippedCheck {
    /// Яка перевірка.
    pub check: CheckId,
    /// Технічна причина (для журналу й звіту).
    pub error: String,
}

/// Результат аудиту.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditReport {
    /// Знахідки, від найвпливовіших.
    pub findings: Vec<Finding>,
    /// Перевірки, що не вдалися.
    pub skipped: Vec<SkippedCheck>,
}

impl AuditReport {
    /// Чи є що виправляти (знахідки вище інформаційного рівня).
    pub fn has_actionable(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.impact > optimax_core::Impact::Info)
    }
}

/// Виконує всі перевірки. Збій однієї не зупиняє інші.
pub fn run_audit(probe: &dyn SystemProbe, today: CivilDate) -> AuditReport {
    let mut findings = Vec::new();
    let mut skipped = Vec::new();
    let mut handle = |check: CheckId, result: Result<Vec<Finding>, CoreError>| match result {
        Ok(f) => findings.extend(f),
        Err(e) => {
            tracing::warn!("перевірка {check:?} не вдалася: {e}");
            skipped.push(SkippedCheck {
                check,
                error: e.to_string(),
            });
        }
    };

    handle(
        CheckId::Firmware,
        probe.firmware().map(|fw| {
            let mut out = rules::check_memory(&fw.memory);
            out.extend(fw.bios.as_ref().map(|b| rules::check_bios(b, today)));
            out
        }),
    );
    handle(
        CheckId::Displays,
        probe.displays().map(|d| rules::check_displays(&d)),
    );
    handle(
        CheckId::Gpus,
        probe.gpus().map(|gpus| {
            gpus.iter()
                .flat_map(|g| {
                    [
                        rules::check_pcie(g),
                        rules::check_rebar(g),
                        rules::check_driver(g, today),
                    ]
                })
                .flatten()
                .collect()
        }),
    );
    handle(
        CheckId::PowerPlan,
        probe
            .power_plan()
            .map(|p| vec![rules::check_power_plan(&p)]),
    );
    handle(
        CheckId::GameMode,
        probe.game_mode().map(|s| vec![rules::check_game_mode(s)]),
    );
    handle(
        CheckId::Hags,
        probe.hags().map(|h| vec![rules::check_hags(&h)]),
    );

    // Стабільне сортування: спершу вплив, потім впевненість; порядок перевірок зберігається.
    findings.sort_by(|a, b| {
        b.impact
            .cmp(&a.impact)
            .then(b.confidence.cmp(&a.confidence))
    });
    AuditReport { findings, skipped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::rules::power_guids;
    use optimax_core::{Catalog, Impact};

    struct MockProbe {
        fail_gpus: bool,
    }

    impl SystemProbe for MockProbe {
        fn firmware(&self) -> Result<FirmwareInfo, CoreError> {
            Ok(FirmwareInfo {
                bios: Some(BiosInfo {
                    vendor: "AMI".into(),
                    version: "F12".into(),
                    release_date: CivilDate::new(2020, 1, 1),
                }),
                memory: vec![MemoryModule {
                    locator: "DIMM_A2".into(),
                    size_mib: 16384,
                    memory_type: MemoryType::Ddr4,
                    rated_mts: Some(3200),
                    configured_mts: Some(2133),
                    manufacturer: "Kingston".into(),
                    part_number: "KF432C16BB/16".into(),
                }],
            })
        }
        fn displays(&self) -> Result<Vec<DisplayInfo>, CoreError> {
            Ok(vec![DisplayInfo {
                device_name: r"\\.\DISPLAY1".into(),
                monitor_name: "Mon".into(),
                primary: true,
                width: 1920,
                height: 1080,
                current_hz: 60,
                max_hz_at_current_res: 144,
            }])
        }
        fn gpus(&self) -> Result<Vec<GpuAdapter>, CoreError> {
            if self.fail_gpus {
                return Err(CoreError::Os {
                    api: "SetupDiGetClassDevsW",
                    code: 5,
                });
            }
            Ok(vec![GpuAdapter {
                name: "GPU".into(),
                vendor: GpuVendor::Nvidia,
                driver_version: Some("32.0.15.6094".into()),
                driver_date: CivilDate::new(2024, 1, 1),
                pcie: Some(PcieLink {
                    current_width: 8,
                    max_width: 16,
                    current_gen: Some(4),
                    max_gen: Some(4),
                }),
                rebar: RebarState::Disabled { bar1_mib: 256 },
                vram_mib: Some(8192),
            }])
        }
        fn power_plan(&self) -> Result<PowerPlan, CoreError> {
            Ok(PowerPlan {
                guid: power_guids::POWER_SAVER.into(),
                name: Some("Економія енергії".into()),
            })
        }
        fn game_mode(&self) -> Result<GameModeState, CoreError> {
            Ok(GameModeState::Disabled)
        }
        fn hags(&self) -> Result<HagsInfo, CoreError> {
            Ok(HagsInfo {
                registry: HagsState::Disabled,
                adapters: None,
            })
        }
    }

    fn today() -> CivilDate {
        CivilDate::new(2026, 10, 22).unwrap()
    }

    #[test]
    fn full_audit_sorted_by_impact() {
        let r = run_audit(&MockProbe { fail_gpus: false }, today());
        assert!(r.skipped.is_empty());
        assert!(r.has_actionable());
        assert!(r.findings.windows(2).all(|w| w[0].impact >= w[1].impact));
        let high: Vec<&str> = r
            .findings
            .iter()
            .filter(|f| f.impact == Impact::High)
            .map(|f| f.id.as_str())
            .collect();
        assert_eq!(
            high,
            vec!["display.low_refresh", "ram.below_rated", "power.saver"]
        );
    }

    #[test]
    fn failed_check_is_reported_not_hidden() {
        let r = run_audit(&MockProbe { fail_gpus: true }, today());
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].check, CheckId::Gpus);
        assert!(r
            .findings
            .iter()
            .all(|f| f.category != optimax_core::Category::Gpu));
    }

    /// Кожен ключ і кожен аргумент шаблону мають бути в каталозі — інакше користувач побачить сирий ключ.
    #[test]
    fn every_finding_text_is_localized() {
        let catalog = Catalog::ukrainian().unwrap();
        let mut texts: Vec<Text> = Vec::new();
        let mut collect = |f: &Finding| {
            texts.push(f.title.clone());
            texts.push(f.detail.clone());
            texts.extend(f.advice.clone());
            for e in &f.evidence {
                texts.push(e.label.clone());
                texts.push(e.value.clone());
            }
        };
        for f in &run_audit(&MockProbe { fail_gpus: false }, today()).findings {
            collect(f);
        }
        // Варіанти, яких немає в моку.
        let extra_probe_findings = [
            rules::check_memory(&[]),
            rules::check_memory(&[MemoryModule {
                locator: "A".into(),
                size_mib: 1,
                memory_type: MemoryType::Ddr4,
                rated_mts: None,
                configured_mts: None,
                manufacturer: String::new(),
                part_number: String::new(),
            }]),
        ];
        for f in extra_probe_findings.iter().flatten() {
            collect(f);
        }
        let variants = [
            rules::check_game_mode(GameModeState::Enabled),
            rules::check_game_mode(GameModeState::Default),
            rules::check_hags(&HagsInfo {
                registry: HagsState::Enabled,
                adapters: None,
            }),
            rules::check_hags(&HagsInfo {
                registry: HagsState::NotSet,
                adapters: None,
            }),
            rules::check_hags(&HagsInfo {
                registry: HagsState::Unexpected(3),
                adapters: None,
            }),
            rules::check_hags(&HagsInfo {
                registry: HagsState::NotSet,
                adapters: Some(vec![HagsAdapter {
                    name: "A".into(),
                    supported: true,
                    enabled: true,
                    enabled_by_default: true,
                }]),
            }),
            rules::check_hags(&HagsInfo {
                registry: HagsState::NotSet,
                adapters: Some(vec![HagsAdapter {
                    name: "A".into(),
                    supported: true,
                    enabled: false,
                    enabled_by_default: false,
                }]),
            }),
            rules::check_hags(&HagsInfo {
                registry: HagsState::Enabled,
                adapters: Some(vec![HagsAdapter {
                    name: "A".into(),
                    supported: true,
                    enabled: false,
                    enabled_by_default: false,
                }]),
            }),
            rules::check_hags(&HagsInfo {
                registry: HagsState::Disabled,
                adapters: Some(vec![HagsAdapter {
                    name: "A".into(),
                    supported: true,
                    enabled: true,
                    enabled_by_default: false,
                }]),
            }),
            rules::check_hags(&HagsInfo {
                registry: HagsState::NotSet,
                adapters: Some(vec![HagsAdapter {
                    name: "A".into(),
                    supported: false,
                    enabled: false,
                    enabled_by_default: false,
                }]),
            }),
            rules::check_power_plan(&PowerPlan {
                guid: power_guids::BALANCED.into(),
                name: None,
            }),
            rules::check_power_plan(&PowerPlan {
                guid: power_guids::HIGH_PERFORMANCE.into(),
                name: None,
            }),
            rules::check_power_plan(&PowerPlan {
                guid: "x".into(),
                name: None,
            }),
        ];
        for f in &variants {
            collect(f);
        }
        let mut g = MockProbe { fail_gpus: false }.gpus().unwrap().remove(0);
        g.pcie = Some(PcieLink {
            current_width: 16,
            max_width: 16,
            current_gen: None,
            max_gen: None,
        });
        g.rebar = RebarState::Enabled { bar1_mib: 8192 };
        g.driver_date = CivilDate::new(2026, 10, 1);
        for f in [
            rules::check_pcie(&g),
            rules::check_rebar(&g),
            rules::check_driver(&g, today()),
        ]
        .iter()
        .flatten()
        {
            collect(f);
        }
        g.rebar = RebarState::Unknown;
        g.driver_date = CivilDate::new(2026, 3, 1);
        for f in [rules::check_rebar(&g), rules::check_driver(&g, today())]
            .iter()
            .flatten()
        {
            collect(f);
        }
        collect(&rules::check_bios(
            &BiosInfo {
                vendor: "A".into(),
                version: "1".into(),
                release_date: CivilDate::new(2026, 1, 1),
            },
            today(),
        ));
        let mut d = MockProbe { fail_gpus: false }.displays().unwrap();
        d[0].current_hz = 144;
        for f in &rules::check_displays(&d) {
            collect(f);
        }
        for c in [
            CheckId::Firmware,
            CheckId::Displays,
            CheckId::Gpus,
            CheckId::PowerPlan,
            CheckId::GameMode,
            CheckId::Hags,
        ] {
            texts.push(c.name());
        }
        texts.push(
            Text::new("audit.skipped")
                .arg("check", "x")
                .arg("error", "y"),
        );

        for t in &texts {
            assert!(catalog.contains(&t.key), "немає ключа в uk.json: {}", t.key);
            let rendered = catalog.render(t);
            assert!(
                !rendered.contains('{'),
                "незаповнений аргумент у {}: {rendered}",
                t.key
            );
        }
    }
}
