// Знайомство з програмою (перший запуск): 3 розділи — «що вміє» → «запиши гру» → «ось результат».
// Кроки підсвічують справжні елементи інтерфейсу (атрибут `data-tour`), пояснення — з каталогу.
import { api } from "./api";
import { ui, type ScreenId } from "./state.svelte";

/** Розділ знайомства (номер у «Крок N з 3»). */
export type Chapter = 1 | 2 | 3;

export interface TourStep {
  /** Ключ кроку: тексти `ui.tour.<id>.title` / `.text`. */
  id: string;
  chapter: Chapter;
  /** Екран, на якому показується крок. */
  screen: ScreenId;
  /** Елемент із `data-tour="…"`; без нього — вікно по центру. */
  target?: string;
  /** Пропускається, якщо елемента немає (напр. радар у прикладі без графіка). */
  optional?: boolean;
}

export const STEPS: TourStep[] = [
  { id: "welcome", chapter: 1, screen: "analysis" },
  { id: "home", chapter: 1, screen: "analysis", target: "home-state" },
  { id: "game", chapter: 2, screen: "diagnosis", target: "diag-game" },
  { id: "duration", chapter: 2, screen: "diagnosis", target: "diag-duration" },
  { id: "start_mode", chapter: 2, screen: "diagnosis", target: "diag-start-mode" },
  { id: "start", chapter: 2, screen: "diagnosis", target: "diag-start" },
  { id: "verdict", chapter: 3, screen: "diagnosis", target: "diag-summary" },
  { id: "numbers", chapter: 3, screen: "diagnosis", target: "diag-stats", optional: true },
  { id: "findings", chapter: 3, screen: "diagnosis", target: "diag-findings" },
  { id: "radar", chapter: 3, screen: "diagnosis", target: "diag-radar", optional: true },
  { id: "help", chapter: 3, screen: "diagnosis", target: "diag-help" },
  { id: "done", chapter: 3, screen: "diagnosis" },
];

export const tour = $state({
  active: false,
  index: 0,
});

/** Поточний крок (або `null`, коли знайомство не йде). */
export function currentStep(): TourStep | null {
  return tour.active ? (STEPS[tour.index] ?? null) : null;
}

/** Розділ 3 без власного запису — екран діагностики показує позначений приклад. */
export function wantsExample(): boolean {
  const s = currentStep();
  return s !== null && s.chapter === 3;
}

function go(index: number) {
  const step = STEPS[index];
  if (!step) {
    finish();
    return;
  }
  tour.index = index;
  ui.screen = step.screen;
}

export function startTour() {
  tour.active = true;
  go(0);
}

export function next() {
  go(tour.index + 1);
}

export function back() {
  if (tour.index > 0) go(tour.index - 1);
}

/** Завершити або пропустити: більше саме не показується (повтор — у налаштуваннях і «?»). */
export function finish() {
  tour.active = false;
  tour.index = 0;
  if (!ui.settings.onboarded) {
    ui.settings.onboarded = true;
    void api.setSettings($state.snapshot(ui.settings)).catch(() => undefined);
  }
}

