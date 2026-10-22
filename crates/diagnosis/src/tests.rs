//! Сценарні тести діагностики на синтетичних записах.

use super::*;
use crate::timeline::{FramePoint, GpuPoint, SamplePoint, Timeline};
use optimax_core::{Catalog, Confidence};
use optimax_frametime::{FrameRecord, FrameTimestamp, Target};
use optimax_metrics::{
    CpuSnapshot, DiskSnapshot, GpuSnapshot, MemorySnapshot, ProcessSample, Snapshot, ThrottleReason,
};
use std::collections::HashMap;

const SAMPLE_MS: f64 = 200.0;

/// Нерівномірні кадри навколо `base` мс (щоб не спрацьовував детектор ліміту FPS).
fn jittery(base: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| base * [0.85, 1.0, 1.2, 0.95, 1.1][i % 5])
        .collect()
}

fn base_sample() -> SamplePoint {
    SamplePoint {
        t_ms: 0.0,
        busiest_core: 0,
        busiest_core_pct: 50.0,
        avg_cpu_pct: 20.0,
        gpu: Some(GpuPoint {
            util: Some(70),
            clock_mhz: Some(2600),
            max_clock_mhz: Some(2800),
            vram_used: Some(4 << 30),
            vram_total: Some(12 << 30),
            temperature_c: Some(65),
            throttle: vec![],
        }),
        disk_bytes_per_sec: 1 << 20,
        background: HashMap::new(),
    }
}

fn timeline(frametimes: &[f64], sample: impl Fn(f64, usize) -> SamplePoint) -> Timeline {
    let mut t = 0.0;
    let frames: Vec<FramePoint> = frametimes
        .iter()
        .map(|&ft| {
            t += ft;
            FramePoint {
                t_ms: t,
                frametime_ms: ft,
                cpu_busy_ms: None,
                gpu_busy_ms: None,
            }
        })
        .collect();
    let n = (t / SAMPLE_MS) as usize + 1;
    let samples = (0..n)
        .map(|i| {
            let ts = i as f64 * SAMPLE_MS;
            let mut s = sample(ts, i);
            s.t_ms = ts;
            s
        })
        .collect();
    Timeline {
        game_pid: 100,
        game_name: "game.exe".into(),
        frames,
        samples,
    }
}

fn ids(d: &Diagnosis) -> Vec<&str> {
    d.verdicts.iter().map(|v| v.finding.id.as_str()).collect()
}

fn find<'a>(d: &'a Diagnosis, id: &str) -> Option<&'a Finding> {
    d.verdicts.iter().map(|v| &v.finding).find(|f| f.id == id)
}

#[test]
fn gpu_bound_scenario() {
    let tl = timeline(&jittery(8.0, 3000), |_, _| {
        let mut s = base_sample();
        if let Some(g) = s.gpu.as_mut() {
            g.util = Some(99);
            g.throttle = vec![ThrottleReason::SwPowerCap];
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    assert_eq!(d.bottleneck, Bottleneck::Gpu);
    assert_eq!(ids(&d)[0], "summary.gpu");
    let f = find(&d, "gpu_bound").unwrap();
    // Без перевірки чутливості до роздільності — не більше 85 %.
    assert!(f.confidence <= Confidence::new(85));
    assert!(find(&d, "cpu_bound").is_none());
    assert!(find(&d, "throttling").is_none(), "power cap — не тротлінг");
    assert!(d.verdicts.iter().all(|v| v.expected_gain.is_none()));
}

#[test]
fn high_util_at_low_clock_is_not_gpu_bound() {
    let tl = timeline(&jittery(8.0, 3000), |_, _| {
        let mut s = base_sample();
        if let Some(g) = s.gpu.as_mut() {
            g.util = Some(99);
            g.clock_mhz = Some(300);
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    assert!(find(&d, "gpu_bound").is_none());
}

#[test]
fn cpu_bound_scenario_with_core_hopping() {
    let tl = timeline(&jittery(12.0, 2000), |_, i| {
        let mut s = base_sample();
        s.busiest_core = [14, 15, 1, 0][i % 4];
        s.busiest_core_pct = 99.0;
        if let Some(g) = s.gpu.as_mut() {
            g.util = Some(55);
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    assert_eq!(d.bottleneck, Bottleneck::Cpu);
    let f = find(&d, "cpu_bound").unwrap();
    assert_eq!(f.impact, Impact::Moderate);
    let hopping = f
        .evidence
        .iter()
        .find(|e| e.label.key == "diag.ev.cores_hopping")
        .unwrap();
    assert_eq!(hopping.value.args["v"], "4");
}

#[test]
fn vsync_cap_is_not_cpu_bound() {
    // 144 Гц, майже ідеальні кадри, одне ядро в стелі (цикл очікування) — як у Geometry Dash.
    let ft: Vec<f64> = (0..3000)
        .map(|i| 6.944 + if i % 2 == 0 { 0.05 } else { -0.05 })
        .collect();
    let tl = timeline(&ft, |_, _| {
        let mut s = base_sample();
        s.busiest_core_pct = 100.0;
        if let Some(g) = s.gpu.as_mut() {
            g.util = Some(35);
        }
        s
    });
    let ctx = Context {
        display_hz: Some(144),
        ..Context::default()
    };
    let d = diagnose_timeline(&tl, &ctx);
    assert_eq!(d.bottleneck, Bottleneck::FrameCap);
    assert!(find(&d, "cap.vsync").is_some());
    assert!(find(&d, "cpu_bound").is_none());
    // Без частоти монітора — загальний «обмежувач FPS».
    let d = diagnose_timeline(&tl, &Context::default());
    assert!(find(&d, "cap.limiter").is_some());
}

/// Кадри по 8 мс із піком 40 мс кожні 250 кадрів (~2 с).
fn spiky() -> Vec<f64> {
    let mut ft = jittery(8.0, 3000);
    for i in (100..3000).step_by(250) {
        ft[i] = 40.0;
    }
    ft
}

fn spike_times(ft: &[f64]) -> Vec<f64> {
    let mut t = 0.0;
    let mut out = Vec::new();
    for &f in ft {
        t += f;
        if f >= 40.0 {
            out.push(t);
        }
    }
    out
}

#[test]
fn stutter_attributed_to_disk() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        if spikes.iter().any(|st| (st - t).abs() <= 150.0) {
            s.disk_bytes_per_sec = 150 << 20;
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.found").expect("фризи з причиною");
    assert!(f.evidence.iter().any(|e| e.label.key == "diag.cause.disk"));
    assert!(!f
        .evidence
        .iter()
        .any(|e| e.label.key == "diag.cause.background"));
}

#[test]
fn stutter_without_cause_is_honest() {
    let tl = timeline(&spiky(), |_, _| base_sample());
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.unknown").expect("фризи без причини");
    assert!(f
        .evidence
        .iter()
        .all(|e| !e.label.key.starts_with("diag.cause.")));
}

/// Як у записі Geometry Dash: кілька ривків на старті, далі рівно.
fn hitches_at_start() -> Vec<f64> {
    let mut ft = jittery(8.0, 3000);
    ft[100] = 60.0;
    ft[115] = 105.0;
    ft[130] = 40.0;
    ft
}

#[test]
fn isolated_hitches_explained_not_called_stutter() {
    let d = diagnose_timeline(
        &timeline(&hitches_at_start(), |_, _| base_sample()),
        &Context::default(),
    );
    assert!(find(&d, "stutter.found").is_none() && find(&d, "stutter.unknown").is_none());
    let f = find(&d, "stutter.isolated").expect("пояснення 0.1% low");
    assert_eq!(f.impact, Impact::Info);
    assert_eq!(f.title.args["spikes"], "3");
    assert_eq!(f.title.args["max"], "105");
}

#[test]
fn background_process_reported_game_excluded() {
    let tl = timeline(&jittery(8.0, 3000), |_, _| {
        let mut s = base_sample();
        s.background.insert("chrome.exe".into(), 13.0);
        s.background.insert("Spotify.exe".into(), 0.5);
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let bg: Vec<&Finding> = d
        .verdicts
        .iter()
        .map(|v| &v.finding)
        .filter(|f| f.id == "background")
        .collect();
    assert_eq!(bg.len(), 1);
    assert_eq!(bg[0].title.args["name"], "chrome.exe");
    assert_eq!(bg[0].title.args["cpu"], "13.0");
}

#[test]
fn thermal_throttling_detected() {
    let tl = timeline(&jittery(8.0, 3000), |_, i| {
        let mut s = base_sample();
        if let Some(g) = s.gpu.as_mut() {
            g.util = Some(98);
            g.temperature_c = Some(88);
            if i % 2 == 0 {
                g.throttle = vec![ThrottleReason::HwThermal];
            }
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "throttling").unwrap();
    assert_eq!(f.impact, Impact::High);
    assert!(f
        .evidence
        .iter()
        .any(|e| e.value.key == "diag.reason.hw_thermal"));
}

#[test]
fn vram_near_full_without_correlation_is_moderate() {
    let tl = timeline(&spiky(), |_, _| {
        let mut s = base_sample();
        if let Some(g) = s.gpu.as_mut() {
            g.vram_used = Some(11_800 << 20);
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "vram.near_full").unwrap();
    assert_eq!(f.impact, Impact::Moderate);
}

#[test]
fn vram_correlated_with_spikes_is_high() {
    // Перша половина: VRAM 9 ГБ і без фризів. Друга: VRAM повна і фризи кожні ~2 с.
    let mut ft = jittery(8.0, 3000);
    for i in (1600..3000).step_by(250) {
        ft[i] = 40.0;
    }
    let half_ms: f64 = ft[..1500].iter().sum();
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        if let Some(g) = s.gpu.as_mut() {
            g.vram_used = Some(if t >= half_ms {
                11_900 << 20
            } else {
                9_000 << 20
            });
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "vram.stutter").unwrap_or_else(|| panic!("{:?}", ids(&d)));
    assert_eq!(f.impact, Impact::High);
}

#[test]
fn insufficient_data() {
    let tl = timeline(&[8.0; 50], |_, _| base_sample());
    let d = diagnose_timeline(&tl, &Context::default());
    assert_eq!(d.bottleneck, Bottleneck::InsufficientData);
    assert_eq!(ids(&d), vec!["summary.insufficient"]);
}

#[test]
fn no_gpu_data_gives_unclear_not_guess() {
    let tl = timeline(&jittery(10.0, 2000), |_, _| {
        let mut s = base_sample();
        s.gpu = None;
        s.busiest_core_pct = 99.0;
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    assert_eq!(d.bottleneck, Bottleneck::Unclear);
    assert!(find(&d, "cpu_bound").is_none());
}

#[test]
fn actionable_config_findings_are_included() {
    let mk = |id: &'static str, impact| {
        FindingBuilder::new("audit", id, Category::Gpu, impact, 70).build()
    };
    let ctx = Context {
        display_hz: None,
        config_findings: vec![
            mk("gpu.rebar_off", Impact::Moderate),
            mk("gpu.pcie_ok", Impact::Info),
        ],
    };
    let d = diagnose_timeline(&timeline(&jittery(8.0, 3000), |_, _| base_sample()), &ctx);
    assert!(find(&d, "gpu.rebar_off").is_some());
    assert!(find(&d, "gpu.pcie_ok").is_none());
}

#[test]
fn every_verdict_text_is_localized() {
    let catalog = Catalog::ukrainian().unwrap();
    let ft = spiky();
    let spikes = spike_times(&ft);
    let scenarios: Vec<(Timeline, Context)> = vec![
        (
            timeline(&jittery(8.0, 3000), |_, _| {
                let mut s = base_sample();
                if let Some(g) = s.gpu.as_mut() {
                    g.util = Some(99);
                    g.throttle = vec![ThrottleReason::SwPowerCap, ThrottleReason::HwThermal];
                    g.vram_used = Some(11_900 << 20);
                }
                s.background.insert("chrome.exe".into(), 13.0);
                s
            }),
            Context::default(),
        ),
        (
            timeline(&jittery(12.0, 2000), |_, _| {
                let mut s = base_sample();
                s.busiest_core_pct = 99.0;
                s
            }),
            Context::default(),
        ),
        (
            timeline(&[6.944; 3000], |_, _| base_sample()),
            Context {
                display_hz: Some(144),
                ..Context::default()
            },
        ),
        (
            timeline(&[6.944; 3000], |_, _| base_sample()),
            Context::default(),
        ),
        (
            timeline(&ft, |t, _| {
                let mut s = base_sample();
                if spikes.iter().any(|st| (st - t).abs() <= 150.0) {
                    s.disk_bytes_per_sec = 150 << 20;
                    s.avg_cpu_pct = 95.0;
                    s.background.insert("updater.exe".into(), 30.0);
                    if let Some(g) = s.gpu.as_mut() {
                        g.throttle = vec![
                            ThrottleReason::SwThermal,
                            ThrottleReason::HwSlowdown,
                            ThrottleReason::HwPowerBrake,
                        ];
                        g.vram_used = Some(11_900 << 20);
                    }
                }
                s
            }),
            Context::default(),
        ),
        (timeline(&spiky(), |_, _| base_sample()), Context::default()),
        (
            timeline(&hitches_at_start(), |_, _| base_sample()),
            Context::default(),
        ),
        (
            timeline(&jittery(10.0, 2000), |_, _| base_sample()),
            Context::default(),
        ),
        (
            timeline(&[8.0; 50], |_, _| base_sample()),
            Context::default(),
        ),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for (tl, ctx) in &scenarios {
        for v in diagnose_timeline(tl, ctx).verdicts {
            let f = v.finding;
            seen.insert(f.id.clone());
            let mut texts = vec![f.title, f.detail];
            texts.extend(f.advice);
            for e in f.evidence {
                texts.push(e.label);
                texts.push(e.value);
            }
            for t in texts {
                assert!(catalog.contains(&t.key), "немає ключа в uk.json: {}", t.key);
                let r = catalog.render(&t);
                assert!(!r.contains('{'), "незаповнений аргумент у {}: {r}", t.key);
            }
        }
    }
    for id in [
        "summary.gpu",
        "summary.cpu",
        "summary.cap",
        "summary.unclear",
        "summary.insufficient",
        "gpu_bound",
        "cpu_bound",
        "cap.vsync",
        "cap.limiter",
        "throttling",
        "stutter.found",
        "stutter.unknown",
        "stutter.isolated",
        "background",
    ] {
        assert!(seen.contains(id), "сценарії не покривають {id}: {seen:?}");
    }
    // Ключі вердиктів, яких немає в сценаріях вище, перевіряємо напряму.
    for key in [
        "diag.vram.stutter.title",
        "diag.vram.near_full.title",
        "diag.summary.insufficient.title",
    ] {
        assert!(catalog.contains(key), "{key}");
    }
}

fn frame(pid: u32, sc: &str, app: &str, qpc: i64, ft: f64) -> FrameRecord {
    FrameRecord {
        application: app.into(),
        process_id: pid,
        swap_chain: sc.into(),
        present_mode: "Hardware: Independent Flip".into(),
        time: FrameTimestamp::Qpc(qpc),
        ms_between_presents: ft,
        ms_between_display_change: None,
        ms_cpu_busy: Some(ft * 0.5),
        ms_gpu_busy: Some(ft * 0.9),
    }
}

fn snapshot(qpc: i64, procs: Vec<(u32, &str, f32)>) -> Snapshot {
    Snapshot {
        elapsed_ms: 0,
        qpc,
        cpu: CpuSnapshot {
            total_percent: 30.0,
            per_core_percent: vec![10.0, 97.0, 20.0],
            per_core_mhz: vec![],
        },
        memory: MemorySnapshot::default(),
        disk: DiskSnapshot {
            read_bytes_per_sec: 100,
            write_bytes_per_sec: 50,
        },
        gpus: vec![
            GpuSnapshot {
                index: 0,
                utilization_percent: Some(5),
                ..GpuSnapshot::default()
            },
            GpuSnapshot {
                index: 1,
                utilization_percent: Some(97),
                vram_used_bytes: Some(1),
                vram_total_bytes: Some(2),
                ..GpuSnapshot::default()
            },
        ],
        top_processes: procs
            .into_iter()
            .map(|(pid, name, cpu)| ProcessSample {
                pid,
                name: name.into(),
                cpu_percent: cpu,
                memory_bytes: 0,
            })
            .collect(),
    }
}

#[test]
fn timeline_from_recording_sorts_filters_and_aligns() {
    let freq = 10_000_000;
    let start = 1_000_000_000;
    let ms = |v: i64| start + v * freq / 1000;
    let rec = Recording {
        target: Target::ProcessName("game.exe".into()),
        qpc_frequency: freq,
        start_qpc: start,
        // Кадри впереміш і з чужим процесом/swap chain, як у реальному виводі PresentMon.
        frames: vec![
            frame(100, "0xA", "game.exe", ms(30), 10.0),
            frame(7, "0xZ", "overlay.exe", ms(15), 10.0),
            frame(100, "0xA", "game.exe", ms(10), 10.0),
            frame(100, "0xA", "game.exe", ms(20), 10.0),
            frame(100, "0xB", "game.exe", ms(25), 10.0),
        ],
        snapshots: vec![
            snapshot(
                ms(400),
                vec![
                    (100, "game.exe", 6.0),
                    (5, "chrome.exe", 3.0),
                    (6, "chrome.exe", 4.0),
                    (9, "PresentMon-2.6.0-x64.exe", 1.0),
                ],
            ),
            snapshot(ms(200), vec![]),
        ],
        skipped_lines: 0,
        presentmon_messages: vec![],
        cancelled: false,
        segments: vec![],
        end_reason: optimax_frametime::EndReason::Completed,
    };
    let tl = Timeline::from_recording(&rec).unwrap();
    assert_eq!(tl.game_pid, 100);
    assert_eq!(tl.game_name, "game.exe");
    let t: Vec<f64> = tl.frames.iter().map(|f| f.t_ms).collect();
    assert_eq!(t, vec![10.0, 20.0, 30.0]);
    assert_eq!(tl.frames[0].gpu_busy_ms, Some(9.0));
    assert_eq!(tl.samples[0].t_ms, 200.0);
    let s = &tl.samples[1];
    assert_eq!(s.t_ms, 400.0);
    assert_eq!((s.busiest_core, s.busiest_core_pct), (1, 97.0));
    // Гра й PresentMon не є «фоном»; кілька chrome.exe сумуються.
    assert_eq!(s.background.len(), 1);
    assert_eq!(s.background["chrome.exe"], 7.0);
    // Обрано GPU, на якому працює гра.
    assert_eq!(s.gpu.as_ref().unwrap().util, Some(97));
    assert_eq!(tl.samples_near(300.0, 100.0).len(), 2);
    assert_eq!(tl.samples_near(300.0, 50.0).len(), 0);
}

#[test]
fn empty_recording_is_insufficient() {
    let rec = Recording {
        target: Target::ProcessId(1),
        qpc_frequency: 1,
        start_qpc: 0,
        frames: vec![],
        snapshots: vec![],
        skipped_lines: 0,
        presentmon_messages: vec![],
        cancelled: false,
        segments: vec![],
        end_reason: optimax_frametime::EndReason::Completed,
    };
    assert_eq!(
        diagnose(&rec, &Context::default()).bottleneck,
        Bottleneck::InsufficientData
    );
}

#[test]
fn frames_outside_counted_segments_are_ignored() {
    // 3 с у меню/Alt+Tab (довгі кадри), потім 20 с гри; зараховано лише гру.
    let freq = 1000;
    let mut frames = Vec::new();
    let mut t = 0i64;
    for _ in 0..30 {
        t += 100;
        frames.push(frame(100, "0xA", "game.exe", t, 100.0)); // 10 FPS у меню
    }
    let game_start = t;
    for _ in 0..2500 {
        t += 8;
        frames.push(frame(100, "0xA", "game.exe", t, 8.0));
    }
    let snaps: Vec<Snapshot> = (0..115).map(|i| snapshot(i * 200, vec![])).collect();
    let rec = Recording {
        target: Target::ProcessName("game.exe".into()),
        qpc_frequency: freq,
        start_qpc: 0,
        frames,
        snapshots: snaps,
        skipped_lines: 0,
        presentmon_messages: vec![],
        cancelled: false,
        segments: vec![(game_start + 1, t + 1)],
        end_reason: optimax_frametime::EndReason::Completed,
    };
    let tl = Timeline::from_recording(&rec).unwrap();
    assert_eq!(tl.frames.len(), 2500);
    assert!(tl.samples.iter().all(|s| s.t_ms >= game_start as f64));
    let d = diagnose(&rec, &Context::default());
    let s = d.stats.unwrap();
    assert!(
        (s.avg_fps - 125.0).abs() < 0.5,
        "меню не тягне FPS вниз: {}",
        s.avg_fps
    );
    assert!(s.low_1pct_fps > 100.0);
}
