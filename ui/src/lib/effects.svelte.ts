// «Ефекти» (docs/DESIGN.md розд. 8, CLAUDE.md 10.5): рівень із налаштувань або за системним
// prefers-reduced-motion, палітра акценту й контракт «під час вимірювань нічого нескінченного».
// Стан віддається через атрибути `<html>`, а компоненти реагують у CSS:
// - `data-effects="calm" | "off"` (Повні — без атрибута);
// - `data-palette="aurora" | "emerald"` (шампань — без атрибута);
// - клас `measuring` — запис, A/B, стабільність, стеження чи активний режим гри;
// - клас `window-hidden` — вікно згорнуте, приховане чи без фокуса (наприклад, поверх нього гра):
//   усі анімації на паузі (docs/DESIGN.md розд. 10: «зупиняються, коли вікно не активне»);
// - клас `idle` — 20 с без миші й клавіатури: нескінченні анімації й фон на паузі до першого руху
//   (заміри етапу 7: кожна велика декоративна анімація — ≈27 % ядра, поки вона йде).
import type { Effects } from "./bindings/Effects";
import type { Palette } from "./bindings/Palette";
import { ui } from "./state.svelte";

const REDUCED = "(prefers-reduced-motion: reduce)";
/** Через скільки без введення декоративний рух зупиняється. */
export const IDLE_MS = 20_000;

const sys = $state({
  reduced: typeof window !== "undefined" && window.matchMedia(REDUCED).matches,
  hidden: typeof document !== "undefined" && document.hidden,
  focused: typeof document === "undefined" || document.hasFocus(),
  idle: false,
});

/** Система просить зменшити рух. */
export function systemReduced(): boolean {
  return sys.reduced;
}

/** Рівень, що діє зараз: вибраний користувачем або за системою (зменшений рух → «Вимкнені»). */
export function effectiveEffects(): Effects {
  return ui.settings.effects ?? (sys.reduced ? "off" : "full");
}

/** Нескінченні анімації й фон зараз заборонені контрактом вимірювань. */
export function measuringNow(): boolean {
  return ui.measuring || ui.gameActive;
}

/** Вікно згорнуте, приховане чи неактивне — анімації й фон стоять. */
export function windowHidden(): boolean {
  return sys.hidden || !sys.focused;
}

/** Користувач давно нічого не робив у вікні — декоративний рух стоїть. */
export function userIdle(): boolean {
  return sys.idle;
}

/** Записує стан в атрибути `<html>` (чиста функція над елементом — для тестів і App). */
export function applyRoot(
  root: HTMLElement,
  s: { effects: Effects; palette: Palette; measuring: boolean; hidden: boolean; idle?: boolean },
): void {
  if (s.effects === "full") delete root.dataset.effects;
  else root.dataset.effects = s.effects;
  if (s.palette === "champagne") delete root.dataset.palette;
  else root.dataset.palette = s.palette;
  root.classList.toggle("measuring", s.measuring);
  root.classList.toggle("window-hidden", s.hidden);
  root.classList.toggle("idle", !!s.idle);
}

/** Стежить за системою й вікном і тримає атрибути `<html>` актуальними. Викликати в `$effect.root`/компоненті. */
export function trackEffects(): () => void {
  const mq = window.matchMedia(REDUCED);
  const onMq = () => (sys.reduced = mq.matches);
  const onVis = () => (sys.hidden = document.hidden);
  const onFocus = () => (sys.focused = true);
  const onBlur = () => (sys.focused = false);
  // Простій: час останнього введення — звичайне число (без реактивного запису на кожен рух миші).
  let lastInput = performance.now();
  const onInput = () => {
    lastInput = performance.now();
    if (sys.idle) sys.idle = false;
  };
  const idleTimer = setInterval(() => {
    if (!sys.idle && performance.now() - lastInput > IDLE_MS) sys.idle = true;
  }, 1000);
  const INPUT = ["pointermove", "pointerdown", "keydown", "wheel"] as const;
  for (const e of INPUT) window.addEventListener(e, onInput, { passive: true, capture: true });
  mq.addEventListener("change", onMq);
  document.addEventListener("visibilitychange", onVis);
  window.addEventListener("focus", onFocus);
  window.addEventListener("blur", onBlur);
  const stop = $effect.root(() => {
    $effect(() =>
      applyRoot(document.documentElement, {
        effects: effectiveEffects(),
        palette: ui.settings.palette ?? "champagne",
        measuring: measuringNow(),
        hidden: windowHidden(),
        idle: sys.idle,
      }),
    );
  });
  return () => {
    stop();
    clearInterval(idleTimer);
    for (const e of INPUT) window.removeEventListener(e, onInput, { capture: true });
    mq.removeEventListener("change", onMq);
    document.removeEventListener("visibilitychange", onVis);
    window.removeEventListener("focus", onFocus);
    window.removeEventListener("blur", onBlur);
  };
}
