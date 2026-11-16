// Навігація «Pro Premium»: 5 вкладок верхньої панелі, у кожній — наявні розділи (docs/DESIGN.md,
// розд. 0). Назви розділів не змінюються; «Програми» й «Вигляд» (ще не готові) не показуються.
import { ui, type ScreenId } from "./state.svelte";

export type TabId = "overview" | "diagnosis" | "tests" | "tools" | "system";

export const TABS: { id: TabId; screens: ScreenId[] }[] = [
  { id: "overview", screens: ["analysis", "hardware", "laptop", "network"] },
  { id: "diagnosis", screens: ["diagnosis"] },
  { id: "tests", screens: ["stability"] },
  { id: "tools", screens: ["optimization", "games", "cleanup", "startup", "upscale"] },
  { id: "system", screens: ["backups", "settings"] },
];

/** Вкладка розділу (розділи поза навігацією — до «Огляду»). */
export function tabOf(screen: ScreenId): TabId {
  return TABS.find((t) => t.screens.includes(screen))?.id ?? "overview";
}

/** Розділи вкладки, де стоїть `screen`. */
export function siblingsOf(screen: ScreenId): ScreenId[] {
  return TABS.find((t) => t.screens.includes(screen))?.screens ?? [];
}

/** Останній відкритий розділ кожної вкладки (повернення на вкладку — туди ж). */
const last = new Map<TabId, ScreenId>();

/** Запам'ятовує поточний розділ (викликається при кожній зміні поточного екрана). */
export function remember(screen: ScreenId): void {
  last.set(tabOf(screen), screen);
}

/** Відкриває вкладку: останній її розділ або перший. */
export function openTab(id: TabId): void {
  const target = last.get(id) ?? TABS.find((t) => t.id === id)?.screens[0];
  if (target) ui.screen = target;
}
