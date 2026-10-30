// Типізовані виклики Tauri-команд і підписки на події. Уся бізнес-логіка — у бекенді;
// тут лише транспорт. Типи — згенеровані з Rust (bindings/).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AbFinal } from "./bindings/AbFinal";
import type { AbView } from "./bindings/AbView";
import type { AppInfo } from "./bindings/AppInfo";
import type { ApplyResult } from "./bindings/ApplyResult";
import type { AuditReport } from "./bindings/AuditReport";
import type { CmdError } from "./bindings/CmdError";
import type { DriveView } from "./bindings/DriveView";
import type { DryRunView } from "./bindings/DryRunView";
import type { ExecProgress } from "./bindings/ExecProgress";
import type { ExecView } from "./bindings/ExecView";
import type { GrowthView } from "./bindings/GrowthView";
import type { JournalRecord } from "./bindings/JournalRecord";
import type { QuarantineView } from "./bindings/QuarantineView";
import type { ScanProgress } from "./bindings/ScanProgress";
import type { ScanView } from "./bindings/ScanView";
import type { DiagnosisResult } from "./bindings/DiagnosisResult";
import type { GameModeEvent } from "./bindings/GameModeEvent";
import type { GameProfile } from "./bindings/GameProfile";
import type { HardwareInfo } from "./bindings/HardwareInfo";
import type { JournalEntry } from "./bindings/JournalEntry";
import type { LogLine } from "./bindings/LogLine";
import type { LaptopView } from "./bindings/LaptopView";
import type { NetProgress } from "./bindings/NetProgress";
import type { NetResult } from "./bindings/NetResult";
import type { NetView } from "./bindings/NetView";
import type { PlanHandle } from "./bindings/PlanHandle";
import type { PowerSchemes } from "./bindings/PowerSchemes";
import type { ProcessEntry } from "./bindings/ProcessEntry";
import type { ProfilePreview } from "./bindings/ProfilePreview";
import type { RecordProgress } from "./bindings/RecordProgress";
import type { RollbackResult } from "./bindings/RollbackResult";
import type { Settings } from "./bindings/Settings";
import type { Snapshot } from "./bindings/Snapshot";
import type { StartupReport } from "./bindings/StartupReport";
import type { Text } from "./bindings/Text";
import type { UiDesired } from "./bindings/UiDesired";
import type { UpscaleView } from "./bindings/UpscaleView";

/** Помилка команди (те, що повертає бекенд, або транспортна). */
export function asCmdError(e: unknown): CmdError {
  if (e && typeof e === "object" && "kind" in e && "message" in e) return e as CmdError;
  return { kind: "failed", message: String(e) };
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  catalog: () => invoke<Record<string, string>>("get_catalog"),
  startupReport: () => invoke<StartupReport>("startup_report"),
  /** Зберігає звіт для відгуку й повертає шлях до файлу. */
  feedbackReport: () => invoke<string>("feedback_report"),
  feedbackReveal: (path: string) => invoke<void>("feedback_reveal", { path }),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),

  runAudit: () => invoke<AuditReport>("run_audit"),
  hardwareInfo: () => invoke<HardwareInfo>("hardware_info"),
  metricsStart: (intervalMs: number) => invoke<void>("metrics_start", { intervalMs }),
  metricsStop: () => invoke<void>("metrics_stop"),

  listProcesses: () => invoke<ProcessEntry[]>("list_processes"),
  diagStart: (exe: string, seconds: number) => invoke<void>("diag_start", { exe, seconds }),
  diagCancel: () => invoke<void>("diag_cancel"),

  powerSchemes: () => invoke<PowerSchemes>("power_schemes"),
  tweakPlan: (desired: UiDesired) => invoke<PlanHandle>("tweak_plan", { desired }),
  tweakApply: (planId: number, allowWithoutRestorePoint: boolean) =>
    invoke<ApplyResult>("tweak_apply", { planId, allowWithoutRestorePoint }),
  journal: () => invoke<JournalEntry[]>("journal_entries"),
  rollback: (id: number) => invoke<void>("journal_rollback", { id }),
  rollbackAll: () => invoke<RollbackResult>("journal_rollback_all"),

  abStart: (game: string, desired: UiDesired, runs: number, seconds: number) =>
    invoke<AbView>("ab_start", { game, desired, runs, seconds }),
  abState: () => invoke<AbView | null>("ab_state"),
  abRunNext: (allowWithoutRestorePoint: boolean) => invoke<AbView>("ab_run_next", { allowWithoutRestorePoint }),
  abFinish: (keep: boolean) => invoke<AbFinal>("ab_finish", { keep }),
  abCancel: () => invoke<void>("ab_cancel"),

  profiles: () => invoke<GameProfile[]>("profiles_list"),
  profilePreview: (profile: GameProfile) => invoke<ProfilePreview>("profile_preview", { profile }),
  profileSave: (profile: GameProfile) => invoke<Text[]>("profile_save", { profile }),
  profileRemove: (exe: string) => invoke<boolean>("profile_remove", { exe }),
  gamemodeRunning: () => invoke<boolean>("gamemode_running"),
  gamemodeStart: () => invoke<void>("gamemode_start"),
  gamemodeStop: () => invoke<void>("gamemode_stop"),
  gamemodeAllowWithoutRestorePoint: () => invoke<void>("gamemode_allow_without_restore_point"),

  upscaleState: () => invoke<UpscaleView>("upscale_state"),
  upscaleStart: (delayS: number) => invoke<void>("upscale_start", { delayS }),
  upscaleCancel: () => invoke<void>("upscale_cancel"),
  upscaleStop: () => invoke<void>("upscale_stop"),

  laptopInfo: () => invoke<LaptopView>("laptop_info"),

  netInfo: () => invoke<NetView>("net_info"),
  netTestStart: (remote: string | null, seconds: number) => invoke<void>("net_test_start", { remote, seconds }),
  netTestCancel: () => invoke<void>("net_test_cancel"),

  cleanupDrives: () => invoke<DriveView[]>("cleanup_drives"),
  cleanupLastScan: () => invoke<ScanView | null>("cleanup_last_scan"),
  cleanupScanStart: (drive: string, full: boolean) => invoke<void>("cleanup_scan_start", { drive, full }),
  cleanupScanCancel: () => invoke<void>("cleanup_scan_cancel"),
  cleanupDryRun: (paths: string[]) => invoke<DryRunView>("cleanup_dry_run", { paths }),
  cleanupExecute: (planId: number, allowWithoutRestorePoint: boolean) =>
    invoke<void>("cleanup_execute", { planId, allowWithoutRestorePoint }),
  cleanupExecCancel: () => invoke<void>("cleanup_exec_cancel"),
  cleanupJournal: () => invoke<JournalRecord[]>("cleanup_journal"),
  cleanupGrowth: (drive: string) => invoke<GrowthView | null>("cleanup_growth", { drive }),
  cleanupQuarantine: () => invoke<QuarantineView>("cleanup_quarantine"),
  cleanupQuarantineRestore: (id: string) => invoke<JournalRecord>("cleanup_quarantine_restore", { id }),
  cleanupQuarantinePurge: (id: string) => invoke<JournalRecord>("cleanup_quarantine_purge", { id }),
  cleanupDism: (allowWithoutRestorePoint: boolean) => invoke<JournalRecord>("cleanup_dism", { allowWithoutRestorePoint }),
  cleanupHibernate: (enable: boolean, allowWithoutRestorePoint: boolean) =>
    invoke<JournalRecord>("cleanup_hibernate", { enable, allowWithoutRestorePoint }),
  cleanupReveal: (path: string) => invoke<void>("cleanup_reveal", { path }),
};

type Events = {
  metrics: Snapshot;
  log: LogLine;
  gamemode: GameModeEvent;
  "diag-progress": RecordProgress;
  "ab-progress": RecordProgress;
  "diag-done": DiagnosisResult;
  "diag-error": CmdError;
  upscale: UpscaleView;
  "net-progress": NetProgress;
  "net-done": NetResult;
  "net-error": CmdError;
  "cleanup-progress": ScanProgress;
  "cleanup-scan-partial": ScanView;
  "cleanup-scan-done": ScanView;
  "cleanup-exec-progress": ExecProgress;
  "cleanup-exec-done": ExecView;
  "cleanup-error": CmdError;
};

/** Підписка на подію бекенду. Повертає функцію відписки. */
export function on<K extends keyof Events>(event: K, cb: (payload: Events[K]) => void): Promise<UnlistenFn> {
  return listen<Events[K]>(event, (e) => cb(e.payload));
}
