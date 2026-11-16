// Форматування записів історії для показу (без логіки порівняння — її рахує бекенд).
import type { StateChange } from "./bindings/StateChange";
import { tk } from "./i18n";

/** FPS цілим числом або «—». */
export const fps = (v: number | null) => (v == null ? "—" : v.toFixed(0));

/** Зміна у відсотках зі знаком або «—». */
export const pct = (v: number | null) => (v == null ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(1)} %`);

/** Стани HAGS / ігрового режиму приходять назвами (`enabled`…) — показуємо словами. */
const STATES = new Set(["enabled", "disabled", "not_set", "default"]);

/** Значення поля стану ПК словами. */
export function stateValue(c: StateChange, v: string): string {
  if ((c.field === "hags" || c.field === "game_mode") && STATES.has(v)) return tk(`ui.history.state.${v}`);
  return v;
}
