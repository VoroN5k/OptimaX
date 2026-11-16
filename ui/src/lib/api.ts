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
import type { ConfigSummary } from "./bindings/ConfigSummary";
import type { DiskSummary } from "./bindings/DiskSummary";
import type { ExpRecord } from "./bindings/ExpRecord";
import type { ExpView } from "./bindings/ExpView";
import type { StabInfo } from "./bindings/StabInfo";
import type { StabRecord } from "./bindings/StabRecord";
import type { StabEntry } from "./bindings/StabEntry";
import type { SensorsStatus } from "./bindings/SensorsStatus";
import type { ServiceView } from "./bindings/ServiceView";
import type { AgentView } from "./bindings/AgentView";
import type { StabView } from "./bindings/StabView";
import type { TestKind } from "./bindings/TestKind";
import type { HistoryView } from "./bindings/HistoryView";
import type { RecordComparison } from "./bindings/RecordComparison";
import type { Regression } from "./bindings/Regression";
import type { DriverCheck } from "./bindings/DriverCheck";
import type { DriverEffect } from "./bindings/DriverEffect";
import type { BlackBoxView } from "./bindings/BlackBoxView";
import type { LastGame } from "./bindings/LastGame";
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
import type { CrashReport } from "./bindings/CrashReport";
import type { DiagnosisState } from "./bindings/DiagnosisState";
import type { ResView } from "./bindings/ResView";
import type { LagView } from "./bindings/LagView";
import type { WatchStatus } from "./bindings/WatchStatus";
import type { Snapshot } from "./bindings/Snapshot";
import type { StartupEntry } from "./bindings/StartupEntry";
import type { StartupReport } from "./bindings/StartupReport";
import type { Text } from "./bindings/Text";
import type { UiDesired } from "./bindings/UiDesired";
import type { UpdateProgress } from "./bindings/UpdateProgress";
import type { UpdateStatus } from "./bindings/UpdateStatus";
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
  /** Записи автозапуску з поточним станом і навантаженням. */
  startupList: () => invoke<StartupEntry[]>("startup_list"),
  /** Перевірка оновлень (запит до GitHub). */
  updateCheck: () => invoke<UpdateStatus>("update_check"),
  /** Завантажує й запускає інсталятор; програма закриється. */
  updateInstall: (version: string) => invoke<void>("update_install", { version }),
  updateOpenPage: () => invoke<void>("update_open_page"),
  feedbackReveal: (path: string) => invoke<void>("feedback_reveal", { path }),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),

  runAudit: () => invoke<AuditReport>("run_audit"),
  dashboardLastGame: () => invoke<LastGame | null>("dashboard_last_game"),
  dashboardConfig: (refresh: boolean) => invoke<ConfigSummary>("dashboard_config", { refresh }),
  dashboardDisk: (refresh: boolean) => invoke<DiskSummary>("dashboard_disk", { refresh }),
  historyList: () => invoke<HistoryView>("history_list"),
  historyCompare: (a: string, b: string) => invoke<RecordComparison>("history_compare", { a, b }),
  historyOpen: (id: string) => invoke<DiagnosisResult>("history_open", { id }),
  historyDelete: (id: string) => invoke<void>("history_delete", { id }),
  historyClear: () => invoke<number>("history_clear"),
  historyRegressions: () => invoke<Regression[]>("history_regressions"),
  historyDriverEffects: () => invoke<DriverEffect[]>("history_driver_effects"),
  driverCheck: () => invoke<DriverCheck>("driver_check"),
  driverOpenPage: (url: string) => invoke<void>("driver_open_page", { url }),
  sensorsStatus: () => invoke<SensorsStatus>("sensors_status"),
  sensorsOpenSite: () => invoke<void>("sensors_open_site"),
  serviceStatus: () => invoke<ServiceView>("service_status"),
  serviceInstall: () => invoke<ServiceView>("service_install"),
  serviceUninstall: () => invoke<ServiceView>("service_uninstall"),
  relaunchElevated: () => invoke<void>("app_relaunch_elevated"),
  agentStatus: () => invoke<AgentView>("agent_status"),
  agentSetAutostart: (on: boolean) => invoke<AgentView>("agent_set_autostart", { on }),
  agentStart: () => invoke<AgentView>("agent_start"),
  blackboxState: () => invoke<BlackBoxView>("blackbox_state"),
  crashReport: () => invoke<CrashReport>("crash_report"),
  hardwareInfo: () => invoke<HardwareInfo>("hardware_info"),
  metricsStart: (intervalMs: number) => invoke<void>("metrics_start", { intervalMs }),
  metricsStop: () => invoke<void>("metrics_stop"),

  listProcesses: () => invoke<ProcessEntry[]>("list_processes"),
  diagStart: (exe: string, seconds: number, scene: string | null) =>
    invoke<void>("diag_start", { exe, seconds, scene }),
  diagCancel: () => invoke<void>("diag_cancel"),
  diagState: () => invoke<DiagnosisState>("diag_state"),
  resStart: (exe: string, seconds: number) => invoke<ResView>("res_start", { exe, seconds }),
  resState: () => invoke<ResView | null>("res_state"),
  resRunNext: () => invoke<ResView>("res_run_next"),
  resCancel: () => invoke<void>("res_cancel"),
  expStart: (exe: string, names: string[], seconds: number, warmup: boolean) =>
    invoke<ExpView>("exp_start", { exe, names, seconds, warmup }),
  expSkipWarmup: () => invoke<ExpView>("exp_skip_warmup"),
  expState: () => invoke<ExpView | null>("exp_state"),
  expRunNext: () => invoke<ExpView>("exp_run_next"),
  expExclude: (index: number, excluded: boolean) => invoke<ExpView>("exp_exclude", { index, excluded }),
  expClose: () => invoke<void>("exp_close"),
  expList: () => invoke<ExpRecord[]>("exp_list"),
  expDelete: (id: string) => invoke<void>("exp_delete", { id }),
  stabInfo: () => invoke<StabInfo>("stab_info"),
  stabStart: (kind: TestKind, seconds: number, acceptNoSensor: boolean) =>
    invoke<StabView>("stab_start", { kind, seconds, acceptNoSensor }),
  stabStop: () => invoke<void>("stab_stop"),
  stabState: () => invoke<StabView | null>("stab_state"),
  stabList: () => invoke<StabEntry[]>("stab_list"),
  stabDelete: (id: string) => invoke<void>("stab_delete", { id }),
  lagStart: (exe: string) => invoke<LagView>("lag_start", { exe }),
  lagState: () => invoke<LagView | null>("lag_state"),
  lagCapture: () => invoke<void>("lag_capture"),
  lagStop: () => invoke<LagView | null>("lag_stop"),
  lagClose: () => invoke<void>("lag_close"),

  powerSchemes: () => invoke<PowerSchemes>("power_schemes"),
  tweakPlan: (desired: UiDesired) => invoke<PlanHandle>("tweak_plan", { desired }),
  tweakApply: (planId: number, allowWithoutRestorePoint: boolean) =>
    invoke<ApplyResult>("tweak_apply", { planId, allowWithoutRestorePoint }),
  journal: () => invoke<JournalEntry[]>("journal_entries"),
  rollback: (id: number) => invoke<void>("journal_rollback", { id }),
  rollbackAll: () => invoke<RollbackResult>("journal_rollback_all"),

  abStart: (game: string, desired: UiDesired, seconds: number) =>
    invoke<AbView>("ab_start", { game, desired, seconds }),
  abStartProfile: (exe: string, seconds: number) => invoke<AbView>("ab_start_profile", { exe, seconds }),
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
  "res-progress": RecordProgress;
  "exp-progress": RecordProgress;
  "exp-update": ExpView;
  "stab-update": StabView;
  "stab-done": StabRecord;
  "lag-status": WatchStatus;
  "lag-update": LagView;
  "diag-done": DiagnosisResult;
  "diag-error": CmdError;
  upscale: UpscaleView;
  "net-progress": NetProgress;
  "net-done": NetResult;
  "net-error": CmdError;
  "cleanup-progress": ScanProgress;
  "cleanup-scan-partial": ScanView;
  "update-progress": UpdateProgress;
  blackbox: BlackBoxView;
  "cleanup-scan-done": ScanView;
  "cleanup-exec-progress": ExecProgress;
  "cleanup-exec-done": ExecView;
  "cleanup-error": CmdError;
};

/** Підписка на подію бекенду. Повертає функцію відписки. */
export function on<K extends keyof Events>(event: K, cb: (payload: Events[K]) => void): Promise<UnlistenFn> {
  return listen<Events[K]>(event, (e) => cb(e.payload));
}
