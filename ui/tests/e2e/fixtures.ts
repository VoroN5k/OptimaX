// Відповіді підробленого бекенду. Типи — ті самі згенеровані прив'язки, що й у інтерфейсі:
// зміна Rust-типу без оновлення тут ламає `npm run check:e2e`.
import type { AppInfo } from "../../src/lib/bindings/AppInfo";
import type { ConfigSummary } from "../../src/lib/bindings/ConfigSummary";
import type { DiagnosisResult } from "../../src/lib/bindings/DiagnosisResult";
import type { DiagnosisState } from "../../src/lib/bindings/DiagnosisState";
import type { DiskSummary } from "../../src/lib/bindings/DiskSummary";
import type { Finding } from "../../src/lib/bindings/Finding";
import type { HistoryEntry } from "../../src/lib/bindings/HistoryEntry";
import type { HistoryView } from "../../src/lib/bindings/HistoryView";
import type { RecordComparison } from "../../src/lib/bindings/RecordComparison";
import type { Regression } from "../../src/lib/bindings/Regression";
import type { DriverCheck } from "../../src/lib/bindings/DriverCheck";
import type { DriverEffect } from "../../src/lib/bindings/DriverEffect";
import type { LagView } from "../../src/lib/bindings/LagView";
import type { LastGame } from "../../src/lib/bindings/LastGame";
import type { ProcessEntry } from "../../src/lib/bindings/ProcessEntry";
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
  };
}
