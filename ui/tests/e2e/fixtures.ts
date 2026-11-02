// Відповіді підробленого бекенду. Типи — ті самі згенеровані прив'язки, що й у інтерфейсі:
// зміна Rust-типу без оновлення тут ламає `npm run check:e2e`.
import type { AppInfo } from "../../src/lib/bindings/AppInfo";
import type { ConfigSummary } from "../../src/lib/bindings/ConfigSummary";
import type { DiagnosisResult } from "../../src/lib/bindings/DiagnosisResult";
import type { DiagnosisState } from "../../src/lib/bindings/DiagnosisState";
import type { DiskSummary } from "../../src/lib/bindings/DiskSummary";
import type { Finding } from "../../src/lib/bindings/Finding";
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
  };
}
