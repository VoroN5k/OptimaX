// Відповіді підробленого бекенду. Типи — ті самі згенеровані прив'язки, що й у інтерфейсі:
// зміна Rust-типу без оновлення тут ламає `npm run check:e2e`.
import type { AppInfo } from "../../src/lib/bindings/AppInfo";
import type { ConfigSummary } from "../../src/lib/bindings/ConfigSummary";
import type { DiagnosisResult } from "../../src/lib/bindings/DiagnosisResult";
import type { DiagnosisState } from "../../src/lib/bindings/DiagnosisState";
import type { DiskSummary } from "../../src/lib/bindings/DiskSummary";
import type { ExpRecord } from "../../src/lib/bindings/ExpRecord";
import type { ExpView } from "../../src/lib/bindings/ExpView";
import type { StabInfo } from "../../src/lib/bindings/StabInfo";
import type { SensorsStatus } from "../../src/lib/bindings/SensorsStatus";
import type { StabEntry } from "../../src/lib/bindings/StabEntry";
import type { StabRecord } from "../../src/lib/bindings/StabRecord";
import type { StabView } from "../../src/lib/bindings/StabView";
import type { Finding } from "../../src/lib/bindings/Finding";
import type { HistoryEntry } from "../../src/lib/bindings/HistoryEntry";
import type { HistoryView } from "../../src/lib/bindings/HistoryView";
import type { RecordComparison } from "../../src/lib/bindings/RecordComparison";
import type { Regression } from "../../src/lib/bindings/Regression";
import type { TimelineView } from "../../src/lib/bindings/TimelineView";
import type { DriverCheck } from "../../src/lib/bindings/DriverCheck";
import type { DriverEffect } from "../../src/lib/bindings/DriverEffect";
import type { BlackBoxView } from "../../src/lib/bindings/BlackBoxView";
import type { LagView } from "../../src/lib/bindings/LagView";
import type { LastGame } from "../../src/lib/bindings/LastGame";
import type { ProcessEntry } from "../../src/lib/bindings/ProcessEntry";
import type { AbFinal } from "../../src/lib/bindings/AbFinal";
import type { AbView } from "../../src/lib/bindings/AbView";
import type { ExpReport } from "../../src/lib/bindings/ExpReport";
import type { ResView } from "../../src/lib/bindings/ResView";
import type { ScanView } from "../../src/lib/bindings/ScanView";
import type { Settings } from "../../src/lib/bindings/Settings";
import type { StartupReport } from "../../src/lib/bindings/StartupReport";
import type { UpscaleView } from "../../src/lib/bindings/UpscaleView";

/** Команди бекенду, які викликає інтерфейс під час тестів, і їхні відповіді. */
export interface Responses {
  app_info: AppInfo;
  get_settings: Settings;
  set_settings: null;
  startup_report: StartupReport;
  upscale_state: UpscaleView;
  dashboard_last_game: LastGame | null;
  dashboard_config: ConfigSummary;
  dashboard_disk: DiskSummary;
  diag_state: DiagnosisState;
  diag_start: null;
  lag_state: LagView | null;
  res_state: ResView | null;
  list_processes: ProcessEntry[];
  cleanup_last_scan: ScanView | null;
  metrics_start: null;
  metrics_stop: null;
  history_list: HistoryView;
  history_compare: RecordComparison;
  history_open: DiagnosisResult;
  history_delete: null;
  history_clear: number;
  history_regressions: Regression[];
  history_driver_effects: DriverEffect[];
  driver_check: DriverCheck;
  driver_open_page: null;
  sensors_status: SensorsStatus;
  sensors_open_site: null;
  blackbox_state: BlackBoxView;
  exp_state: ExpView | null;
  exp_list: ExpRecord[];
  stab_info: StabInfo;
  stab_state: StabView | null;
  stab_list: StabEntry[];
  stab_start: StabView;
  stab_stop: null;
  stab_delete: null;
  exp_start: ExpView;
  exp_run_next: ExpView;
  exp_close: null;
  ab_state: AbView | null;
  ab_finish: AbFinal;
}

const GIB = 1073741824;

export function finding(f: Partial<Finding> & Pick<Finding, "id" | "title" | "detail">): Finding {
  return { category: "gpu", impact: "info", confidence: 80, advice: null, evidence: [], ...f };
}

export const settings: Settings = {
  animation: "off",
  economy: false,
  record_start: "auto",
  record_sound: true,
  upscale: { filter: "fsr", fit: "aspect", sharpness: 0.2, capture_cursor: false },
  upscale_hotkey: false,
  cleanup: { quarantine_days: 7, quarantine_drive: null, auto_purge: false },
  update_check_on_start: false,
  driver_check_on_start: false,
  power_off_by_button: false,
  overlay: false,
  onboarded: true,
  pawnio_sensors: false,
};

export const config: ConfigSummary = {
  high: 1,
  moderate: 1,
  driver: finding({
    id: "driver.aging",
    category: "driver",
    title: { key: "audit.driver.aging.title", args: { days: "190" } },
    detail: { key: "audit.driver.aging.detail", args: { gpu: "GeForce RTX 4070", version: "566.36", date: "2026-04-26" } },
  }),
  report: {
    findings: [
      finding({
        id: "ram.single_module",
        category: "memory",
        impact: "high",
        title: { key: "audit.ram.single_module.title", args: {} },
        detail: { key: "audit.ram.single_module.detail", args: { size_gb: "16" } },
        advice: { key: "audit.ram.single_module.advice", args: { slots: "4" } },
      }),
      finding({
        id: "driver.aging",
        category: "driver",
        impact: "moderate",
        title: { key: "audit.driver.aging.title", args: { days: "190" } },
        detail: { key: "audit.driver.aging.detail", args: { gpu: "GeForce RTX 4070", version: "566.36", date: "2026-04-26" } },
      }),
    ],
    skipped: [],
  },
};

export const disk: DiskSummary = {
  drive: "C:\\",
  safe_bytes: 12.3 * GIB,
  consequences_bytes: 4 * GIB,
  free_bytes: 120 * GIB,
  total_bytes: 930 * GIB,
  scanned_unix: 1_793_000_000,
};

export const lastGame: LastGame = {
  game: "re9.exe",
  bottleneck: "gpu",
  title: { key: "diag.summary.gpu.title", args: { game: "re9.exe" } },
  confidence: 81,
  avg_fps: 125,
  low_1pct_fps: 57.2,
  at_unix: Math.floor(Date.now() / 1000) - 2 * 86400,
};

/** Справжній (не приклад) результат запису: відеокарта, дві знахідки. */
/** Шкала запису на 30 с: кадри по 8 мс, один ривок 60 мс близько 12,5 с, DPC поруч. */
function syntheticView(): TimelineView {
  const t: number[] = [];
  const ft: number[] = [];
  let at = 1000;
  while (at < 31000) {
    const v = at >= 12500 && at < 12508 ? 60 : 8;
    at += v;
    t.push(at);
    ft.push(v);
  }
  return {
    start_ms: t[0] ?? 0,
    end_ms: t[t.length - 1] ?? 0,
    median_ms: 8,
    frames: { t, ft, cpu: ft.map((v) => v * 0.7), gpu: ft.map(() => 7.4), queue: ft.map(() => 3.2) },
    spikes: [[12564, 60]],
    gaps: [],
    samples: { t: [2000, 12000, 22000], gpu: [97, 95, 96], core: [55, 60, 58] },
    main: {
      tid: 1,
      start_ms: 1000,
      bucket_ms: 250,
      buckets: Array.from({ length: 120 }, () => [180, 10, 50, 0, 10, 0] as [number, number, number, number, number, number]),
      detail: [[12504, 18, 0], [12522, 36, 3], [12558, 6, 0]],
    },
    render: null,
    dpc: [{ t: 12528, us: 1200, driver: 0, isr: false }],
    drivers: ["nvlddmkm.sys"],
    shader_writes: [],
  };
}

export const gpuResult: DiagnosisResult = {
  diagnosis: {
    game: "re9.exe",
    stats: {
      frames: 3751,
      duration_ms: 30000,
      avg_fps: 125,
      low_1pct_fps: 57.2,
      low_01pct_fps: 16.5,
      median_frametime_ms: 7.9,
      p99_frametime_ms: 10.99,
      max_frametime_ms: 60,
      spikes: 4,
    },
    bottleneck: "gpu",
    verdicts: [
      {
        finding: finding({
          id: "summary.gpu",
          category: "summary",
          confidence: 81,
          title: { key: "diag.summary.gpu.title", args: { game: "re9.exe" } },
          detail: {
            key: "diag.summary.gpu.detail",
            args: { seconds: "30", frames: "3751", avg: "125.0", low1: "57.2", low01: "16.5" },
          },
        }),
        expected_gain: null,
      },
      {
        finding: finding({
          id: "gpu_bound",
          confidence: 85,
          title: { key: "diag.gpu_bound.title", args: { share: "97" } },
          detail: { key: "diag.gpu_bound.detail", args: {} },
          advice: { key: "diag.gpu_bound.advice", args: {} },
        }),
        expected_gain: null,
      },
      {
        finding: finding({
          id: "upscale.helps",
          confidence: 85,
          title: { key: "diag.upscale.helps.title", args: {} },
          detail: { key: "diag.upscale.helps.detail", args: {} },
          advice: { key: "diag.upscale.helps.advice", args: {} },
        }),
        expected_gain: null,
      },
    ],
    chart: null,
    frame_gen: null,
    latency: {
      frame_to_display: { median_ms: 18.4, p95_ms: 27.1, samples: 3700 },
      input_to_photon: { median_ms: 31.2, p95_ms: 44.6, samples: 812 },
      click_to_photon: null,
      pc_latency: null,
      cpu_busy_ms: 6.1,
      gpu_start_ms: 3.2,
      until_displayed_ms: 12.3,
      frames: 3751,
    },
    anatomy: {
      typical: { frame_ms: 8.0, cpu_busy: 6.1, cpu_wait: 1.9, gpu_latency: 3.2, gpu_busy: 7.4, gpu_wait: 0.3, until_displayed: 4.0 },
      frames: 3751,
      queued_share: 0.34,
      spikes: [
        {
          t_ms: 12500,
          parts: { frame_ms: 60, cpu_busy: 55.2, cpu_wait: 1.1, gpu_latency: 50.0, gpu_busy: 7.0, gpu_wait: 0.5, until_displayed: 5.0 },
          prev_gpu_busy: 7.2,
          stall: "cpu_busy",
          main: { running: 18.0, ready: 1.5, sync: 2.5, page_in: 36.0, sleep: 0, other: 0 },
          main_state: "page_in",
          render: null,
        },
      ],
      stall_counts: [1, 0, 0],
      main_counts: [0, 0, 0, 1, 0, 0],
    },
    view: syntheticView(),
  },
  display_hz: 200,
  presentmon_messages: [],
  cancelled: false,
  counted_s: 30,
  end_reason: "completed",
};

export function historyEntry(at: number, avg: number, driver: string, game = "re9.exe"): HistoryEntry {
  return {
    id: `${at}-${game}`,
    game,
    at_unix_ms: at,
    bottleneck: "gpu",
    title: { key: "diag.summary.gpu.title", args: { game } },
    confidence: 85,
    avg_fps: avg,
    low_1pct_fps: avg * 0.7,
    low_01pct_fps: avg * 0.5,
    frames: 6000,
    counted_s: 60,
    fps_noise_pct: 0.8,
    state: {
      optimax: "0.1.6",
      windows: "Windows 11 Pro",
      windows_build: "26300",
      gpu: "NVIDIA GeForce RTX 4070 SUPER",
      driver,
      display: { width: 2560, height: 1440, hz: 200 },
      power_plan: "Висока продуктивність",
      bios: "F12",
      ram_mts: 6000,
      hags: "enabled",
      game_mode: "enabled",
      game_file: { bytes: 104857600, modified_unix: 1_790_000_000 },
    },
    has_recording: true,
    scene: "коридор",
  };
}

const older = historyEntry(1_793_000_000_000, 125, "566.36");
const newer = historyEntry(1_793_500_000_000, 108, "572.16");

export const history: HistoryView = {
  rows: [
    { entry: newer, changes: [{ field: "driver", from: "566.36", to: "572.16" }] },
    { entry: historyEntry(1_793_200_000_000, 140, "566.36", "cs2.exe"), changes: [] },
    { entry: older, changes: [] },
  ],
  broken: 0,
};

export const comparison: RecordComparison = {
  older,
  newer,
  avg_fps_change_pct: -13.6,
  low_1pct_change_pct: -13.6,
  threshold_pct: 8,
  trend: "worse",
  same_game: true,
  same_display: true,
  scene: "same",
  bottleneck_changed: false,
  changes: [{ field: "driver", from: "566.36", to: "572.16" }],
};

export const driverCheck: Extract<DriverCheck, { kind: "checked" }> = {
  kind: "checked",
  gpu: "NVIDIA GeForce RTX 4070 SUPER",
  installed: "566.36",
  latest: "621.01",
  released: "2026-10-21",
  page: "https://www.nvidia.com/en-us/drivers/details/282412/",
  newer: true,
};

const expRun = (order: number, variant: number, avg: number, suspect: "level" | null = null) => ({
  pc_latency_ms: null as number | null,
  variant,
  order,
  avg_fps: avg,
  low_1pct_fps: avg * 0.62,
  frames: Math.round(avg * 30),
  duration_s: 30,
  low_precision_pct: 2.4,
  frame_capped: false,
  excluded: false,
  suspect,
});

/** Порівняння щойно почалось: прогрів пропущено, перший прогін — A. */
export const expReady: ExpView = {
  game: "re9.exe",
  names: ["DLSS Якість", "DLSS Баланс"],
  seconds: 30,
  start: "auto",
  phase: "ready",
  warmup: null,
  report: {
    status: "continue",
    next: 0,
    variants: 2,
    comparisons: [{ variant: 1, verdict: "unclear", avg_fps: null, low_1pct: null, pc_latency: null }],
    more_per_arm: null,
    confidence_pct: 98.58,
    margin_avg_pct: 1,
    margin_low_pct: 2,
    margin_latency_pct: null,
    runs: [],
    warnings: [],
  },
  max_per_arm: 8,
  saved_id: null,
};

/** Висновок після 3 + 3 прогонів: «Баланс» дає +6 %, один прогін підозрілий. */
export const stabInfo: StabInfo = {
  gpu: "NVIDIA GeForce RTX 4070 SUPER",
  cpu: "AMD Ryzen 7 5800X 8-Core Processor",
  cpu_cores: 8,
  ram_mb: 32768,
  ram_test_mb: 12000,
  has_sensor: true,
  limit_c: 85,
  durations_s: [120, 600, 1800],
  night_durations_s: [14400, 28800],
};

export const stabRunning: StabView = {
  id: "1793700000000-gpu",
  kind: "gpu",
  bad_cores: [],
  current_core: null,
  total_cores: 0,
  started_unix_ms: 1793700000000,
  adapter: "NVIDIA GeForce RTX 4070 SUPER",
  planned_s: 600,
  elapsed_s: 75,
  rounds: 30,
  checked: 26,
  mismatches: 0,
  tested_mb: 2048,
  vram_mb: 12282,
  temp_c: 71,
  clock_mhz: 2610,
  power_w: 205,
  util_pct: 99,
  limit_c: 85,
  stats: { max_temp_c: 71, avg_clock_mhz: 2600, avg_power_w: 200, avg_util_pct: 98 },
  no_sensor: false,
};

export const stabDone: StabRecord = {
  id: "1793700000000-gpu",
  kind: "gpu",
  bad_cores: [],
  last_core: null,
  total_cores: 0,
  started_unix_ms: 1793700000000,
  adapter: "NVIDIA GeForce RTX 4070 SUPER",
  planned_s: 600,
  elapsed_s: 600,
  rounds: 240,
  checked: 236,
  mismatches: 0,
  tested_mb: 2048,
  vram_mb: 12282,
  outcome: "no_errors",
  events: { read: true, tdr: 0, whea_corrected: 0, whea_fatal: 0, unexpected_off: 0 },
  stats: { max_temp_c: 74, avg_clock_mhz: 2600, avg_power_w: 205, avg_util_pct: 99 },
  low_load: false,
  no_sensor: false,
};

export const expDone: ExpView = {
  ...expReady,
  phase: "done",
  saved_id: "1793700000000-re9.exe",
  report: {
    status: "done",
    next: null,
    variants: 2,
    comparisons: [
      {
        variant: 1,
        verdict: "better",
        avg_fps: {
      a: 120,
      b: 127.2,
      delta_pct: 6.0,
      ci_low_pct: 3.1,
      ci_high_pct: 8.9,
      drift_pct_per_run: -0.4,
      run_sd_pct: 0.9,
      outcome: "better",
        },
        low_1pct: {
          a: 74,
          b: 77,
          delta_pct: 4.1,
          ci_low_pct: -1.5,
          ci_high_pct: 9.7,
          drift_pct_per_run: -0.3,
          run_sd_pct: 2.1,
          outcome: "unclear",
        },
        pc_latency: null,
      },
    ],
    more_per_arm: null,
    confidence_pct: 98.58,
    margin_avg_pct: 1,
    margin_low_pct: 2,
    margin_latency_pct: null,
    runs: [
      expRun(0, 0, 120.1),
      expRun(1, 1, 127.5),
      expRun(2, 1, 126.9),
      expRun(3, 0, 119.6),
      expRun(4, 0, 108.0, "level"),
      expRun(5, 1, 127.3),
    ],
    warnings: [{ key: "exp.warn.suspect_level", args: { n: "5", arm: "A" } }],
  },
};

/** Оцінка рушія «зміна проти базового» з одним порівнянням. */
function report2(
  runs: [number, number][],
  done: boolean,
  delta: number,
  lo: number,
  hi: number,
  outcome: "better" | "worse" | "no_effect" | "unclear",
  margin = 1,
): ExpReport {
  const est = (d: number, l: number, h: number) => ({
    a: 100,
    b: 100 + d,
    delta_pct: d,
    ci_low_pct: l,
    ci_high_pct: h,
    drift_pct_per_run: 0.2,
    run_sd_pct: 1.1,
    outcome,
  });
  return {
    status: done ? "done" : "continue",
    next: done ? null : 0,
    variants: 2,
    comparisons: [
      {
        variant: 1,
        verdict: done ? outcome : "unclear",
        avg_fps: est(delta, lo, hi),
        low_1pct: est(delta, lo - 2, hi + 2),
        pc_latency: null,
      },
    ],
    more_per_arm: done ? null : 1,
    confidence_pct: 98.58,
    margin_avg_pct: margin,
    margin_low_pct: margin * 2,
    margin_latency_pct: null,
    runs: runs.map(([variant, avg], order) => expRun(order, variant, avg)),
    warnings: [],
  };
}

/** Тест роздільності після трьох прогонів (A B B): далі — назад на рідну. */
export const resMid: ResView = {
  game: "re9.exe",
  seconds: 30,
  start: "auto",
  runs: [
    { reduced: false, avg_fps: 100, low_1pct_fps: 70, gpu_util_avg: 99, frame_capped: false, frames: 3000 },
    { reduced: true, avg_fps: 138, low_1pct_fps: 95, gpu_util_avg: 85, frame_capped: false, frames: 4140 },
    { reduced: true, avg_fps: 137, low_1pct_fps: 94, gpu_util_avg: 85, frame_capped: false, frames: 4110 },
  ],
  next_reduced: false,
  max_per_arm: 6,
  estimate: report2(
    [
      [0, 100],
      [1, 138],
      [1, 137],
    ],
    false,
    37.5,
    20.1,
    54.9,
    "better",
    3,
  ),
  running: false,
  report: null,
};

/** A/B схеми живлення: 3 + 3 прогони, ефекту немає. */
export const abDone: AbView = {
  game: "re9.exe",
  plan: {
    info: {
      id: "power_scheme",
      title: { key: "tweak.power_scheme.title", args: {} },
      what: { key: "tweak.power_scheme.what", args: {} },
      performance: { key: "tweak.power_scheme.performance", args: {} },
      heat: { key: "tweak.power_scheme.heat", args: {} },
      safety: { key: "tweak.power_scheme.safety", args: {} },
      restart_game_between_runs: false,
      requires_reboot: false,
    },
    changes: [],
  },
  seconds: 60,
  start: "auto",
  report: report2(
    [
      [0, 120.1],
      [1, 120.4],
      [1, 119.8],
      [0, 120.3],
      [0, 119.9],
      [1, 120.2],
    ],
    true,
    0.1,
    -0.6,
    0.8,
    "no_effect",
  ),
  max_per_arm: 8,
  profile: false,
};

export function defaults(): Responses {
  return {
    app_info: { version: "0.0.0-test", elevated: true, presentmon_version: "2.6.0" },
    get_settings: structuredClone(settings),
    set_settings: null,
    startup_report: { recovered_pending: 0, restored_gamemode: { rolled_back: [], failed: [] }, error: null },
    upscale_state: { hotkey: false, hotkey_error: null, countdown: null, running: null, notice: null },
    dashboard_last_game: structuredClone(lastGame),
    dashboard_config: structuredClone(config),
    dashboard_disk: structuredClone(disk),
    diag_state: { recording: false, last: null },
    diag_start: null,
    lag_state: null,
    res_state: null,
    list_processes: [{ name: "re9.exe", cpu_percent: 31, memory_mib: 9000 }],
    cleanup_last_scan: null,
    metrics_start: null,
    metrics_stop: null,
    history_list: structuredClone(history),
    history_compare: structuredClone(comparison),
    history_open: structuredClone(gpuResult),
    history_delete: null,
    history_clear: 3,
    history_regressions: [],
    history_driver_effects: [],
    driver_check: structuredClone(driverCheck),
    driver_open_page: null,
    sensors_status: { state: "not_installed" },
    sensors_open_site: null,
    blackbox_state: { active: null, last: null },
    exp_state: null,
    exp_list: [],
    stab_info: structuredClone(stabInfo),
    stab_state: null,
    stab_list: [],
    stab_start: structuredClone(stabRunning),
    stab_stop: null,
    stab_delete: null,
    exp_start: structuredClone(expReady),
    exp_run_next: structuredClone(expDone),
    exp_close: null,
    ab_state: null,
    ab_finish: { report: structuredClone(abDone.report), markdown: "### OptimaX: порівняння", kept: false },
  };
}
