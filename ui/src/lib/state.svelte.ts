// Глобальний стан інтерфейсу (руни Svelte 5).
import type { AppInfo } from "./bindings/AppInfo";
import type { LogLine } from "./bindings/LogLine";
import type { RollbackResult } from "./bindings/RollbackResult";
import type { Settings } from "./bindings/Settings";
import type { Text } from "./bindings/Text";
import type { UpscaleView } from "./bindings/UpscaleView";

export type ScreenId =
  | "analysis" | "optimization" | "games" | "diagnosis"
  | "hardware" | "laptop" | "network"
  | "upscale" | "cleanup" | "apps" | "startup" | "appearance"
  | "backups" | "settings";

/** Стан фонової анімації (10.4): спокій, сканування, знайдено проблему, успіх. */
export type BgMode = "idle" | "scanning" | "warn" | "ok";

/** Не більше рядків журналу в DOM. */
export const LOG_LIMIT = 500;

export const ui = $state({
  screen: "analysis" as ScreenId,
  info: null as AppInfo | null,
  settings: { animation: "high", economy: false } as Settings,
  bgMode: "idle" as BgMode,
  /** Режим гри активний (гра запущена) — анімація повністю вимикається. */
  gameActive: false,
  /** Стежник режиму гри працює. */
  gamemodeWatching: false,
  /** Іде запис frametime / A/B-бенчмарк — анімація вимикається. */
  measuring: false,
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
