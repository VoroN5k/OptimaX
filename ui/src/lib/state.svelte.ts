// Глобальний стан інтерфейсу (руни Svelte 5).
import type { UpdateStatus } from "./bindings/UpdateStatus";
import type { DriverCheck } from "./bindings/DriverCheck";
import type { DiagnosisResult } from "./bindings/DiagnosisResult";
import type { StoryOutcome } from "./bindings/StoryOutcome";
import type { CheckChoice } from "./bindings/CheckChoice";
import type { AppInfo } from "./bindings/AppInfo";
import type { ExecProgress } from "./bindings/ExecProgress";
import type { ExecView } from "./bindings/ExecView";
import type { ScanProgress } from "./bindings/ScanProgress";
import type { ScanView } from "./bindings/ScanView";
import type { LogLine } from "./bindings/LogLine";
import type { RollbackResult } from "./bindings/RollbackResult";
import type { Settings } from "./bindings/Settings";
import type { Text } from "./bindings/Text";
import type { UpscaleView } from "./bindings/UpscaleView";

export type ScreenId =
  | "analysis" | "optimization" | "games" | "diagnosis"
  | "hardware" | "laptop" | "network"
  | "stability" | "upscale" | "cleanup" | "apps" | "startup" | "appearance"
  | "backups" | "settings";

/** Стан фонової анімації (10.4): спокій, сканування, знайдено проблему, успіх. */
export type BgMode = "idle" | "scanning" | "warn" | "ok";

/** Не більше рядків журналу в DOM. */
export const LOG_LIMIT = 500;

/** Стан сценарію «Перевірити мою гру». */
export interface CheckSession {
  /** Запис сценарію почато (наступний `history-saved` — «було», якщо «було» ще немає). */
  armed: boolean;
  /** Гра, тривалість і сцена першого запису — для повторного. */
  exe: string;
  seconds: number;
  scene: string;
  /** Id запису «було» в «Історії». */
  baseline: string | null;
  /** Підсумок розповіді першого запису (чи є що перевіряти). */
  outcome: StoryOutcome | null;
  /** Іде (чи йшов останнім) повторний запис «стало». */
  rechecking: boolean;
  /** Id останнього збереженого запису «стало». */
  afterId: string | null;
  /** Обрана порада («Спробувати»); `null` — головна дія. */
  choice: CheckChoice | null;
  /** Порада головної дії «було» (щоб на «стало» показати, що саме перевіряємо). */
  mainAdvice: Text | null;
}

export const ui = $state({
  screen: "analysis" as ScreenId,
  info: null as AppInfo | null,
  settings: { effects: null, palette: "champagne", background: "on" } as Settings,
  bgMode: "idle" as BgMode,
  /** Режим гри активний (гра запущена) — анімація повністю вимикається. */
  gameActive: false,
  /** Стежник режиму гри працює. */
  gamemodeWatching: false,
  /** Іде запис frametime / A/B-бенчмарк — анімація вимикається. */
  measuring: false,
  /** Результат останньої перевірки оновлень (за кнопкою або при запуску, якщо дозволено). */
  update: null as UpdateStatus | null,
  /** Результат перевірки драйвера на nvidia.com (за кнопкою або при запуску, якщо дозволено). */
  driverCheck: null as DriverCheck | null,
  /** Мітка сцени для наступного запису (живе, поки відкрита програма). */
  scene: "",
  /** Порівняти ці записи історії, щойно відкриється вкладка «Історія» (з «Головної»). */
  pendingCompare: null as [string, string] | null,
  /** Підсумок цієї сесії чорної скриньки сховано (до перезапуску програми). */
  blackboxHidden: null as string | null,
  /** З «Головної» натиснули «Встановити службу»: картка служби одразу покаже згоду. */
  serviceInstall: false,
  /** Останній запис гри (для фонового шару з реальних даних). */
  lastRecord: null as DiagnosisResult | null,
  /** Id останнього запису в історії (приходить після `diag-done`); `null` — ще не збережено. */
  lastRecordId: null as string | null,
  /** Сценарій «Перевірити мою гру» (з «Огляду»); `null` — звичайна «Діагностика». Лише сесія. */
  check: null as CheckSession | null,
  /** Відкрита висувна панель журналу дій (кнопка «Журнал» у верхній панелі). */
  logOpen: false,
});

export const log = $state({ lines: [] as LogLine[] });

/** Додає рядок у журнал дій, обрізаючи до [`LOG_LIMIT`]. */
export function pushLog(line: LogLine): void {
  log.lines.push(line);
  if (log.lines.length > LOG_LIMIT) log.lines.splice(0, log.lines.length - LOG_LIMIT);
}

/** Локальний рядок журналу (дії інтерфейсу). */
export function logLocal(level: "info" | "warn" | "error", message: string): void {
  pushLog({ level, target: "optimax_ui", message, unix_ms: Date.now() });
}

/** Коротка «хвиля» анімації (warn/ok), далі — повернення до спокою. */
let flashTimer: ReturnType<typeof setTimeout> | undefined;
export function flashBg(mode: "warn" | "ok"): void {
  ui.bgMode = mode;
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => {
    if (ui.bgMode === mode) ui.bgMode = "idle";
  }, 1600);
}


/** Стан режиму гри (з подій бекенду `gamemode`). */
export const gm = $state({
  watching: false,
  games: [] as string[],
  active: null as { game: string; pid: number; applied: Text[]; skipped: Text[] } | null,
  lastEnded: null as { game: string; result: RollbackResult } | null,
  needsConfirmation: null as { game: string; reason: string } | null,
});

/** Стан апскейлу (подія бекенду `upscale`). */
export const up = $state({ view: null as UpscaleView | null });

/**
 * Очищення (події `cleanup-*`): живе поза екраном, щоб скан чи очищення не «губилися» при
 * переході між розділами.
 */
export const cl = $state({
  scanning: false,
  progress: null as ScanProgress | null,
  view: null as ScanView | null,
  executing: false,
  execProgress: null as ExecProgress | null,
  lastExec: null as ExecView | null,
  error: null as string | null,
});
