//! Сценарні тести діагностики на синтетичних записах.

use super::*;
use crate::framegen::FrameGenSeries;
use crate::timeline::{FramePoint, GpuPoint, SamplePoint, ShaderWrite, Timeline};
use optimax_core::{Catalog, Confidence};
use optimax_frametime::{FrameRecord, FrameTimestamp, PresentMode, Target};
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
            fan_pct: Some(55),
            power_limit_w: Some(220.0),
            default_power_limit_w: Some(220.0),
        }),
        disk_bytes_per_sec: 1 << 20,
        background: HashMap::new(),
        mem_available_fraction: Some(0.5),
        hard_faults_per_sec: Some(10.0),
        dpc_pct: Some(0.5),
        io_top: None,
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
                present_mode: PresentMode::HardwareIndependentFlip,
                sync_interval: Some(0),
                allows_tearing: Some(true),
                display_ms: None,
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
        shader_writes: Vec::new(),
        dpc: None,
    }
}

/// Перші `share` кадрів — у режимі `mode`, решта лишається як була.
fn with_mode(mut tl: Timeline, mode: PresentMode, share: f64) -> Timeline {
    let n = (tl.frames.len() as f64 * share) as usize;
    for f in tl.frames.iter_mut().take(n) {
        f.present_mode = mode;
        f.allows_tearing = Some(false);
    }
    tl
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
    // Апскейл радимо з тією самою впевненістю, що й головний висновок, і без обіцянок приросту.
    let up = find(&d, "upscale.helps").unwrap();
    assert_eq!(up.confidence, f.confidence);
    assert!(up.advice.is_some());
    assert!(find(&d, "upscale.no_help").is_none());
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
    // Чесно: апскейл тут не допоможе.
    assert!(find(&d, "upscale.no_help").is_some());
    assert!(find(&d, "upscale.helps").is_none());
}

#[test]
fn no_upscale_advice_without_clear_bottleneck() {
    for b in [
        Bottleneck::FrameCap,
        Bottleneck::Unclear,
        Bottleneck::InsufficientData,
    ] {
        assert!(upscale_advice(b, 80).is_none());
    }
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

fn near_spike(spikes: &[f64], t: f64) -> bool {
    spikes.iter().any(|st| (st - t).abs() <= 150.0)
}

fn cause_keys(f: &Finding) -> Vec<&str> {
    f.evidence
        .iter()
        .map(|e| e.label.key.as_str())
        .filter(|k| k.starts_with("diag.cause."))
        .collect()
}

#[test]
fn defender_scan_is_its_own_cause_not_generic_background() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        if near_spike(&spikes, t) {
            s.background.insert("MsMpEng.exe".into(), 6.0);
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.found").expect("фризи з причиною");
    assert_eq!(cause_keys(f), vec!["diag.cause.defender"]);
    assert!(f
        .evidence
        .iter()
        .any(|e| e.value.key == "diag.advice.defender"));
}

#[test]
fn defender_with_constant_load_gets_own_background_verdict() {
    let tl = timeline(&jittery(8.0, 3000), |_, _| {
        let mut s = base_sample();
        s.background.insert("MsMpEng.exe".into(), 7.0);
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    assert!(find(&d, "background.defender").is_some());
    assert!(find(&d, "background").is_none());
}

#[test]
fn memory_cause_needs_paging_and_nearly_full_ram() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    let run = |free: f64| {
        let tl = timeline(&ft, |t, _| {
            let mut s = base_sample();
            s.mem_available_fraction = Some(free);
            if near_spike(&spikes, t) {
                s.hard_faults_per_sec = Some(4000.0);
            }
            s
        });
        diagnose_timeline(&tl, &Context::default())
    };
    let low = run(0.06);
    let f = find(&low, "stutter.found").expect("нестача RAM");
    assert_eq!(cause_keys(f), vec!["diag.cause.memory"]);
    // Ті самі page faults при вільній пам'яті — це читання файлів, а не нестача RAM.
    let plenty = run(0.5);
    assert!(find(&plenty, "stutter.found").is_none());
    assert!(find(&plenty, "stutter.unknown").is_some());
}

#[test]
fn shader_cache_writes_near_spikes_point_to_shader_compilation() {
    // Ривки від компіляції шейдерів ідуть згустками (нова сцена), між згустками тихо. Тож
    // вікно запису кешу (−200…+1500 мс) не накриває весь запис і «біля ривків частіше, ніж
    // зазвичай» має сенс. Якби драйвер писав кеш постійно, приписувати ривки шейдерам не
    // можна — це перевіряє правило вдвічі частішого збігу.
    let mut ft = jittery(8.0, 5000);
    for cluster in (400..5000).step_by(1200) {
        for k in 0..15 {
            ft[cluster + k * 6] = 40.0;
        }
    }
    let spikes = spike_times(&ft);
    let mut tl = timeline(&ft, |_, _| base_sample());
    tl.shader_writes = spikes
        .iter()
        .map(|t| ShaderWrite {
            t_ms: t + 300.0,
            bytes: 64 << 10,
        })
        .collect();
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.found").expect("компіляція шейдерів");
    assert_eq!(cause_keys(f), vec!["diag.cause.shader"]);
    assert!(
        f.confidence.percent() <= 65,
        "непряма ознака — помірна впевненість"
    );
    assert!(f
        .evidence
        .iter()
        .any(|e| e.label.key == "diag.ev.shader_cache"));
}

#[test]
fn shader_cache_files_are_placed_on_the_recording_timeline() {
    let start = 1_700_000_000_000;
    let files = [
        CacheFile {
            modified_unix_ms: start + 5_000,
            bytes: 10,
        },
        CacheFile {
            modified_unix_ms: start - 10,
            bytes: 20,
        },
        CacheFile {
            modified_unix_ms: start + 1_000,
            bytes: 30,
        },
        CacheFile {
            modified_unix_ms: start + 90_000,
            bytes: 40,
        },
    ];
    let w = shader_writes(&files, start, 30_000.0);
    let got: Vec<(f64, u64)> = w.iter().map(|x| (x.t_ms, x.bytes)).collect();
    assert_eq!(got, vec![(1_000.0, 30), (5_000.0, 10)]);
}

#[test]
fn chart_keeps_every_spike_visible_and_explains_it() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        if near_spike(&spikes, t) {
            s.disk_bytes_per_sec = 150 << 20;
        }
        s
    });
    let chart = diagnose_timeline(&tl, &Context::default())
        .chart
        .expect("графік");
    assert!(chart.points.len() <= chart::CHART_POINTS);
    // Стиснення не губить піків: найвища точка графіка — 40 мс.
    let top = chart.points.iter().map(|p| p.max_ms).fold(0.0, f32::max);
    assert!((top - 40.0).abs() < 1e-3);
    assert_eq!(chart.spikes.len(), spikes.len());
    assert_eq!(chart.spikes_total as usize, spikes.len());
    assert!(chart.spikes.windows(2).all(|w| w[0].t_ms <= w[1].t_ms));
    for s in &chart.spikes {
        assert!((s.frametime_ms - 40.0).abs() < 1e-9);
        assert_eq!(s.causes.len(), 1);
        assert_eq!(s.causes[0].key, "diag.spike.disk");
        assert_eq!(s.causes[0].args["mbps"], "150");
    }
}

#[test]
fn disk_spike_names_the_busiest_process() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        if near_spike(&spikes, t) {
            s.disk_bytes_per_sec = 90 << 20;
            s.io_top = Some(crate::timeline::IoProcess {
                name: "ForzaHorizon4.exe".into(),
                bytes_per_sec: 80 << 20,
            });
        }
        s
    });
    let chart = diagnose_timeline(&tl, &Context::default())
        .chart
        .expect("графік");
    let c = &chart.spikes[0].causes[0];
    assert_eq!(c.key, "diag.spike.disk_by");
    assert_eq!(c.args["name"], "ForzaHorizon4.exe");
    assert_eq!(c.args["mbps"], "90");
    assert_eq!(c.args["pmbps"], "80");
}

/// Як у записі RE9: диск 43 МБ/с, а найактивніший процес у замірі — 2 МБ/с. Називати його
/// не можна: справжній сплеск у замір процесів не потрапив.
#[test]
fn disk_spike_does_not_blame_a_minor_process() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        if near_spike(&spikes, t) {
            s.disk_bytes_per_sec = 43 << 20;
            s.io_top = Some(crate::timeline::IoProcess {
                name: "msedgewebview2.exe".into(),
                bytes_per_sec: 2 << 20,
            });
        }
        s
    });
    let chart = diagnose_timeline(&tl, &Context::default())
        .chart
        .expect("графік");
    let c = &chart.spikes[0].causes[0];
    assert_eq!(c.key, "diag.spike.disk");
    assert_eq!(c.args["mbps"], "43");
}

/// DLSS FG ×2: Present() парами (0.5 мс + 17.5 мс), на екрані — рівно по 9 мс; кілька
/// справжніх ривків (на екрані 40 мс).
fn framegen_timeline(with_display: bool) -> Timeline {
    let n = 4000;
    let ft: Vec<f64> = (0..n)
        .map(|i| if i % 2 == 0 { 0.5 } else { 17.5 })
        .collect();
    let mut tl = timeline(&ft, |_, _| base_sample());
    if with_display {
        for (i, f) in tl.frames.iter_mut().enumerate() {
            f.display_ms = Some(if i % 400 == 201 { 40.0 } else { 9.0 });
        }
    }
    tl
}

#[test]
fn frame_generation_is_measured_by_what_is_shown() {
    let d = diagnose_timeline(&framegen_timeline(true), &Context::default());
    let fg = d.frame_gen.as_ref().expect("генерацію кадрів виявлено");
    assert_eq!(fg.series, FrameGenSeries::Display);
    assert!((fg.factor - 2.0).abs() < 0.05, "{fg:?}");
    assert!((fg.real_fps - 55.6).abs() < 1.0, "{fg:?}");
    // Вердикт про генерацію — одразу після підсумку.
    assert_eq!(d.verdicts[1].finding.id, "framegen.display");
    // Ривки — лише справжні (10), а не кожен другий кадр.
    let chart = d.chart.as_ref().unwrap();
    assert_eq!(chart.spikes_total, 10);
    let stats = d.stats.as_ref().unwrap();
    assert!(stats.avg_fps > 100.0, "на екрані ~111 FPS: {stats:?}");
    assert!(find(&d, "stutter.unknown").is_none() && find(&d, "stutter.found").is_none());
}

#[test]
fn frame_generation_without_display_times_uses_real_frames() {
    let d = diagnose_timeline(&framegen_timeline(false), &Context::default());
    let fg = d.frame_gen.as_ref().unwrap();
    assert_eq!(fg.series, FrameGenSeries::Real);
    assert_eq!(d.verdicts[1].finding.id, "framegen.real");
    let stats = d.stats.as_ref().unwrap();
    assert!((stats.avg_fps - 55.6).abs() < 1.0, "{stats:?}");
    assert_eq!(d.chart.as_ref().unwrap().spikes_total, 0);
}

#[test]
fn ordinary_games_are_not_mistaken_for_frame_generation() {
    for tl in [
        timeline(&jittery(8.0, 3000), |_, _| base_sample()),
        timeline(&spiky(), |_, _| base_sample()),
        timeline(&hitches_at_start(), |_, _| base_sample()),
    ] {
        assert!(diagnose_timeline(&tl, &Context::default())
            .frame_gen
            .is_none());
    }
}

#[test]
fn downsample_keeps_max_and_average() {
    let frames = [(10.0, 10.0), (20.0, 30.0), (110.0, 10.0), (120.0, 20.0)];
    let p = chart::downsample(&frames, 2);
    assert_eq!(p.len(), 2);
    // Шкала від першого кадру, а не від нуля (до нього — очікування гри й прогрів).
    assert_eq!(p[0].t_ms, 10.0);
    assert_eq!(p[0].max_ms, 30.0);
    assert_eq!(p[0].avg_ms, 20.0);
    assert_eq!(p[1].max_ms, 20.0);
    assert_eq!(p[1].avg_ms, 15.0);
    assert!(chart::downsample(&[], 10).is_empty());
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
                g.clock_mhz = Some(2300);
            }
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "throttling").unwrap();
    // 2300 замість 2600 МГц — втрата ~12 %, виміряна: високий вплив.
    assert_eq!(f.impact, Impact::High);
    assert!(f
        .evidence
        .iter()
        .any(|e| e.value.key == "diag.ev.throttle_clock_value"));
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
fn unclear_summary_shows_load_against_thresholds() {
    // GPU 91 %: «майже завжди 90+», але до порогу 95 % не дотягує — вердикт «не ясно».
    let tl = timeline(&jittery(8.0, 3000), |_, i| {
        let mut s = base_sample();
        if let Some(g) = s.gpu.as_mut() {
            g.util = Some(if i % 4 == 0 { 96 } else { 91 });
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    assert_eq!(d.bottleneck, Bottleneck::Unclear);
    let head = &d.verdicts[0].finding;
    let ev = |key: &str| {
        head.evidence
            .iter()
            .find(|e| e.label.key == key)
            .unwrap_or_else(|| panic!("немає {key}"))
    };
    let avg: f64 = ev("diag.ev.gpu_avg").value.args["v"].parse().unwrap();
    assert!((91.0..=93.0).contains(&avg), "{avg}");
    let share = ev("diag.ev.gpu_busy_share");
    assert_eq!(share.value.key, "value.pct_needed");
    let v: f64 = share.value.args["v"].parse().unwrap();
    assert!((20.0..=30.0).contains(&v), "{v}");
    assert_eq!(share.value.args["need"], "60");
    let core = ev("diag.ev.core_saturated_share");
    assert_eq!(core.value.args["v"], "0");
    assert_eq!(core.value.args["need"], "50");
}

#[test]
fn clear_verdict_summary_has_no_load_evidence() {
    let tl = timeline(&jittery(8.0, 3000), |_, _| {
        let mut s = base_sample();
        if let Some(g) = s.gpu.as_mut() {
            g.util = Some(99);
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    assert_eq!(d.bottleneck, Bottleneck::Gpu);
    assert!(d.verdicts[0].finding.evidence.iter().all(
        |e| !e.label.key.starts_with("diag.ev.gpu") && !e.label.key.starts_with("diag.ev.core")
    ));
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
        shader_cache_files: Vec::new(),
    };
    let d = diagnose_timeline(&timeline(&jittery(8.0, 3000), |_, _| base_sample()), &ctx);
    assert!(find(&d, "gpu.rebar_off").is_some());
    assert!(find(&d, "gpu.pcie_ok").is_none());
}

#[test]
fn composed_present_reported_with_evidence() {
    let tl = with_mode(
        timeline(&jittery(8.0, 3000), |_, _| base_sample()),
        PresentMode::ComposedFlip,
        0.9,
    );
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "present.composed").unwrap();
    assert_eq!(f.category, Category::Display);
    assert!(f.advice.is_some());
    // Прямий факт PresentMon: 70 + 25 × 0.9.
    assert_eq!(f.confidence, Confidence::new(93));
    let ev = |key: &str| {
        f.evidence
            .iter()
            .find(|e| e.label.key == key)
            .map(|e| e.value.clone())
    };
    assert_eq!(
        ev("diag.ev.present_mode").unwrap(),
        Text::new("value.raw").arg("v", "Composed: Flip")
    );
    assert!(ev("diag.ev.sync_interval").is_some());
    assert_eq!(
        ev("diag.ev.allows_tearing").unwrap(),
        Text::new("value.pct").arg("v", "0")
    );
    assert!(d.verdicts.iter().all(|v| v.expected_gain.is_none()));
}

#[test]
fn direct_or_mixed_present_is_not_reported() {
    let base = || timeline(&jittery(8.0, 3000), |_, _| base_sample());
    let reported = |tl: &Timeline| {
        find(
            &diagnose_timeline(tl, &Context::default()),
            "present.composed",
        )
        .is_some()
    };
    // Напряму (Independent Flip, апаратний оверлей) — нічого не кажемо.
    assert!(!reported(&base()));
    assert!(!reported(&with_mode(
        base(),
        PresentMode::HardwareComposedIndependentFlip,
        1.0
    )));
    // Композиція лише частину часу (менше 60 %) — не переважна.
    assert!(!reported(&with_mode(
        base(),
        PresentMode::ComposedFlip,
        0.4
    )));
    // Режим невідомий у більшості кадрів (колонку вимкнено) — без висновку, навіть якщо
    // решта кадрів складена DWM.
    let mut unknown = with_mode(base(), PresentMode::Other, 0.7);
    for f in unknown.frames.iter_mut().skip(2100) {
        f.present_mode = PresentMode::ComposedFlip;
    }
    assert!(!reported(&unknown));
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
                    s.io_top = Some(crate::timeline::IoProcess {
                        name: "game.exe".into(),
                        bytes_per_sec: 100 << 20,
                    });
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
        (
            with_mode(
                timeline(&jittery(8.0, 3000), |_, _| base_sample()),
                PresentMode::ComposedCopyGpuGdi,
                1.0,
            ),
            Context::default(),
        ),
        (
            // Нові причини разом: Defender, нестача RAM, кеш шейдерів (див. нижче).
            timeline(&ft, |t, _| {
                let mut s = base_sample();
                s.mem_available_fraction = Some(0.05);
                if spikes.iter().any(|st| (st - t).abs() <= 150.0) {
                    s.background.insert("MsMpEng.exe".into(), 6.0);
                    s.hard_faults_per_sec = Some(5000.0);
                }
                s
            }),
            Context::default(),
        ),
        (
            timeline(&jittery(8.0, 3000), |_, _| {
                let mut s = base_sample();
                s.background.insert("MsMpEng.exe".into(), 7.0);
                s
            }),
            Context::default(),
        ),
        (framegen_timeline(true), Context::default()),
        (framegen_timeline(false), Context::default()),
    ];
    let scenarios: Vec<(Timeline, Context)> = scenarios
        .into_iter()
        .enumerate()
        .map(|(i, (mut tl, ctx))| {
            // Сценарій з усіма новими причинами отримує й записи в кеш шейдерів.
            if i == 10 {
                tl.shader_writes = spikes
                    .iter()
                    .map(|t| ShaderWrite {
                        t_ms: t + 300.0,
                        bytes: 1 << 20,
                    })
                    .collect();
            }
            (tl, ctx)
        })
        .collect();
    // Тексти біля ривків на графіку теж мають бути перекладені.
    for (tl, ctx) in &scenarios {
        if let Some(chart) = diagnose_timeline(tl, ctx).chart {
            for t in chart.spikes.iter().flat_map(|s| &s.causes) {
                assert!(catalog.contains(&t.key), "немає ключа в uk.json: {}", t.key);
                let r = catalog.render(t);
                assert!(!r.contains('{'), "незаповнений аргумент у {}: {r}", t.key);
            }
        }
    }
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
        "background.defender",
        "framegen.display",
        "framegen.real",
        "present.composed",
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
        sync_interval: None,
        allows_tearing: None,
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
            dpc_percent: None,
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
                io_bytes_per_sec: 0,
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
        start_unix_ms: None,
        dpc_trace: None,
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
        start_unix_ms: None,
        dpc_trace: None,
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
        start_unix_ms: None,
        dpc_trace: None,
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

#[test]
fn dpc_spikes_are_a_cause_with_capped_confidence() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        if near_spike(&spikes, t) {
            s.dpc_pct = Some(12.0);
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.found").expect("фризи з причиною");
    assert_eq!(cause_keys(f), vec!["diag.cause.dpc"]);
    assert!(f.confidence <= Confidence::new(70), "DPC — непряма ознака");
    assert!(f.evidence.iter().any(|e| e.value.key == "diag.advice.dpc"));
    let chart = d.chart.as_ref().unwrap();
    assert_eq!(chart.summary[0].name.key, "diag.cause_name.dpc");
    assert_eq!(chart.unexplained, 0);
}

fn dpc_trace(long: Vec<crate::timeline::DpcPoint>) -> crate::timeline::DpcTimeline {
    let driver = |name: &str, max_us: f64| optimax_core::etw::DriverDpc {
        name: Some(name.to_string()),
        dpc_count: 1000,
        isr_count: 0,
        total_us: 10_000.0,
        max_us,
    };
    crate::timeline::DpcTimeline {
        drivers: vec![driver("nvlddmkm.sys", 1500.0), driver("ntoskrnl.exe", 60.0)],
        long,
        lost: 0,
    }
}

fn stall(t_ms: f64, dur_us: f64, driver: u16) -> crate::timeline::DpcPoint {
    crate::timeline::DpcPoint {
        t_ms,
        dur_us,
        driver,
        isr: false,
    }
}

#[test]
fn traced_driver_stalls_name_the_driver() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    // Лічильник PDH спокійний (секундне усереднення ховає 1,5 мс), але трасування бачить
    // довге виконання nvlddmkm перед кожним ривком; короткі виконання — шум, не ознака.
    let mut tl = timeline(&ft, |_, _| base_sample());
    let mut long: Vec<_> = spikes.iter().map(|t| stall(t - 20.0, 1500.0, 0)).collect();
    long.extend((0..50).map(|i| stall(f64::from(i) * 300.0 + 7.0, 150.0, 1)));
    long.sort_by(|a, b| a.t_ms.total_cmp(&b.t_ms));
    tl.dpc = Some(dpc_trace(long));
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.found").expect("фризи з причиною");
    assert_eq!(cause_keys(f), vec!["diag.cause.dpc"]);
    assert!(
        f.confidence > Confidence::new(70),
        "драйвер названо трасуванням"
    );
    assert!(f.confidence <= Confidence::new(85));
    let driver = f
        .evidence
        .iter()
        .find(|e| e.label.key == "diag.ev.dpc_driver")
        .expect("драйвер біля ривків");
    assert_eq!(driver.value.key, "diag.ev.dpc_driver_value");
    assert!(driver.value.args.iter().any(|(_, v)| v == "nvlddmkm.sys"));
    assert!(f
        .evidence
        .iter()
        .any(|e| e.value.key == "diag.driver_kind.gpu"));
    assert!(f
        .evidence
        .iter()
        .any(|e| e.value.key == "diag.advice.dpc_gpu"));
    assert!(f
        .evidence
        .iter()
        .any(|e| e.label.key == "diag.ev.dpc_longest"));
    let chart = d.chart.as_ref().unwrap();
    assert!(chart
        .spikes
        .iter()
        .any(|m| m.causes.iter().any(|c| c.key == "diag.spike.dpc_driver")));
}

#[test]
fn constant_driver_stalls_are_background() {
    let ft = spiky();
    // Довгі виконання драйвера рівномірно весь запис — біля піків не частіше, ніж зазвичай.
    let mut tl = timeline(&ft, |_, _| base_sample());
    let end = tl.frames.last().map(|f| f.t_ms).unwrap_or(0.0);
    let n = (end / 50.0) as u32;
    tl.dpc = Some(dpc_trace(
        (0..n)
            .map(|i| stall(f64::from(i) * 50.0, 900.0, 0))
            .collect(),
    ));
    let d = diagnose_timeline(&tl, &Context::default());
    assert!(find(&d, "stutter.found").is_none());
    let f = find(&d, "stutter.unknown").expect("причину не знайдено");
    assert!(f
        .evidence
        .iter()
        .any(|e| e.label.key == "diag.cause_name.dpc" && e.value.key == "diag.ev.as_usual"));
    assert!(!f
        .evidence
        .iter()
        .any(|e| e.label.key == "diag.ev.dpc_driver"));
}

#[test]
fn small_dpc_noise_is_not_a_cause() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    // Нижче абсолютного порога: звичайне коливання навіть без навантаження.
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        s.dpc_pct = Some(if near_spike(&spikes, t) { 3.0 } else { 0.3 });
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    assert!(find(&d, "stutter.found").is_none());
}

#[test]
fn mixed_causes_give_partial_verdict_and_summary() {
    let ft = spiky();
    let spikes = spike_times(&ft);
    // Третина ривків — з диском, решта — без ознак.
    let tl = timeline(&ft, |t, _| {
        let mut s = base_sample();
        if spikes
            .iter()
            .enumerate()
            .any(|(i, st)| i % 3 == 0 && (st - t).abs() <= 150.0)
        {
            s.disk_bytes_per_sec = 150 << 20;
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.partial").expect("частково пояснені фризи");
    assert!(f
        .evidence
        .iter()
        .any(|e| e.label.key == "diag.cause_name.disk" && e.value.key == "diag.ev.partial_share"));
    let chart = d.chart.as_ref().unwrap();
    let disk = chart
        .summary
        .iter()
        .find(|c| c.name.key == "diag.cause_name.disk")
        .unwrap();
    assert_eq!(disk.spikes + chart.unexplained, chart.spikes.len() as u32);
    assert!(disk.spikes > 0 && chart.unexplained > 0);
}

/// Кадри з V-Sync на `hz`: здебільшого один інтервал, кожен `every`-й — два.
fn vsync_frames(hz: f64, every: usize, n: usize) -> Vec<f64> {
    let iv = 1000.0 / hz;
    (0..n)
        .map(|i| {
            if i % every == 0 {
                iv * 2.0
            } else {
                iv * [0.98, 1.0, 1.02][i % 3]
            }
        })
        .collect()
}

fn vsync_on(mut tl: Timeline) -> Timeline {
    for f in &mut tl.frames {
        f.sync_interval = Some(1);
        f.allows_tearing = Some(false);
    }
    tl
}

#[test]
fn vsync_judder_detected_only_with_vsync_and_known_refresh() {
    let ctx = Context {
        display_hz: Some(60),
        ..Context::default()
    };
    let tl = vsync_on(timeline(&vsync_frames(60.0, 8, 2000), |_, _| base_sample()));
    let d = diagnose_timeline(&tl, &ctx);
    assert!(find(&d, "vsync.judder").is_some());
    // Без частоти монітора — не вгадуємо.
    assert!(find(&diagnose_timeline(&tl, &Context::default()), "vsync.judder").is_none());
    // V-Sync вимкнено (SyncInterval 0) — шаблон не про нього.
    let off = timeline(&vsync_frames(60.0, 8, 2000), |_, _| base_sample());
    assert!(find(&diagnose_timeline(&off, &ctx), "vsync.judder").is_none());
    // Рівні 60 FPS без пропусків — це просто ліміт, а не ривки.
    let even = vsync_on(timeline(&vsync_frames(60.0, usize::MAX, 2000), |_, _| {
        base_sample()
    }));
    assert!(find(&diagnose_timeline(&even, &ctx), "vsync.judder").is_none());
    // З VRR кадри не кратні оновленню.
    let vrr = vsync_on(timeline(&jittery(22.0, 2000), |_, _| base_sample()));
    assert!(find(&diagnose_timeline(&vrr, &ctx), "vsync.judder").is_none());
}

#[test]
fn periodic_spikes_found_only_for_regular_intervals() {
    // spiky(): пік кожні 250 кадрів — однаковий інтервал.
    let tl = timeline(&spiky(), |_, _| base_sample());
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.periodic").expect("періодичні ривки");
    assert_eq!(f.evidence[0].label.key, "diag.ev.spike_period");
    // Ті самі ривки з довільними інтервалами.
    let mut ft = jittery(8.0, 3000);
    for i in [100, 180, 520, 610, 1200, 1290, 1900, 2500, 2560, 2900] {
        ft[i] = 40.0;
    }
    let tl = timeline(&ft, |_, _| base_sample());
    assert!(find(
        &diagnose_timeline(&tl, &Context::default()),
        "stutter.periodic"
    )
    .is_none());
}

#[test]
fn constant_disk_activity_is_background_not_cause() {
    // Гра часто підвантажує світ: сплески диска йдуть і біля ривків, і без них (поріг сплеску
    // рахується від медіани, тож сплески мають бути менш ніж у половині знімків).
    let tl = timeline(&spiky(), |_, i| {
        let mut s = base_sample();
        if i % 5 < 2 {
            s.disk_bytes_per_sec = 150 << 20;
        }
        s
    });
    let d = diagnose_timeline(&tl, &Context::default());
    let f = find(&d, "stutter.unknown").expect("диск — тло, причину не встановлено");
    assert!(f
        .evidence
        .iter()
        .any(|e| e.label.key == "diag.cause_name.disk" && e.value.key == "diag.ev.as_usual"));
    let chart = d.chart.as_ref().unwrap();
    let disk = chart.summary.iter().find(|c| c.code == "disk").unwrap();
    assert!(!disk.above_usual);
    assert!(disk.usual > 0.5);
}

fn throttled(clock_on: u32, fan: u32, limit: f32, every: usize) -> Diagnosis {
    let tl = timeline(&jittery(8.0, 3000), |_, i| {
        let mut s = base_sample();
        if let Some(g) = s.gpu.as_mut() {
            g.util = Some(98);
            g.temperature_c = Some(89);
            g.fan_pct = Some(fan);
            g.power_limit_w = Some(limit);
            if i % every != 0 {
                g.throttle = vec![ThrottleReason::SwThermal];
                g.clock_mhz = Some(clock_on);
            }
        }
        s
    });
    diagnose_timeline(&tl, &Context::default())
}

#[test]
fn throttling_impact_follows_measured_clock_loss() {
    // Частота майже та сама — тротлінг є, але майже нічого не забирає.
    let small = throttled(2580, 60, 220.0, 3);
    assert_eq!(find(&small, "throttling").unwrap().impact, Impact::Info);
    // Тротлінг майже весь час — порівняти нема з чим: помірний вплив і чесне «не виміряно».
    let always = throttled(2300, 60, 220.0, usize::MAX);
    let f = find(&always, "throttling").unwrap();
    assert_eq!(f.impact, Impact::Moderate);
    assert!(f
        .evidence
        .iter()
        .any(|e| e.value.key == "diag.ev.throttle_clock_unknown"));
}

#[test]
fn throttling_explains_low_fan_and_raised_power_limit() {
    let d = throttled(2400, 30, 253.0, 3);
    let f = find(&d, "throttling").unwrap();
    let values: Vec<&str> = f.evidence.iter().map(|e| e.value.key.as_str()).collect();
    assert!(values.contains(&"diag.advice.fan_low"));
    assert!(values.contains(&"diag.ev.power_limit_value"));
    assert!(values.contains(&"diag.advice.power_raised"));
    // Звичайні оберти й заводський ліміт — без цих порад.
    let d = throttled(2400, 70, 220.0, 3);
    let f = find(&d, "throttling").unwrap();
    assert!(
        !f.evidence
            .iter()
            .any(|e| e.value.key == "diag.advice.fan_low"
                || e.value.key == "diag.advice.power_raised")
    );
}
