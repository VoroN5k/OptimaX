// «Ефекти» (docs/DESIGN.md розд. 8, CLAUDE.md 10.5): рівень із налаштувань або за системним
// prefers-reduced-motion, палітра акценту й контракт «під час вимірювань нічого нескінченного».
// Стан віддається через атрибути `<html>`, а компоненти реагують у CSS:
// - `data-effects="calm" | "off"` (Повні — без атрибута);
// - `data-palette="aurora" | "emerald"` (шампань — без атрибута);
// - клас `measuring` — запис, A/B, стабільність, стеження чи активний режим гри;
// - клас `window-hidden` — вікно згорнуте чи приховане: усі анімації на паузі.
import type { Effects } from "./bindings/Effects";
import type { Palette } from "./bindings/Palette";
import { ui } from "./state.svelte";

const REDUCED = "(prefers-reduced-motion: reduce)";

const sys = $state({
  reduced: typeof window !== "undefined" && window.matchMedia(REDUCED).matches,
  hidden: typeof document !== "undefined" && document.hidden,
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

/** Вікно згорнуте чи приховане. */
export function windowHidden(): boolean {
  return sys.hidden;
}

/** Записує стан в атрибути `<html>` (чиста функція над елементом — для тестів і App). */
export function applyRoot(
  root: HTMLElement,
  s: { effects: Effects; palette: Palette; measuring: boolean; hidden: boolean },
): void {
  if (s.effects === "full") delete root.dataset.effects;
  else root.dataset.effects = s.effects;
  if (s.palette === "champagne") delete root.dataset.palette;
  else root.dataset.palette = s.palette;
  root.classList.toggle("measuring", s.measuring);
  root.classList.toggle("window-hidden", s.hidden);
}

/** Стежить за системою й вікном і тримає атрибути `<html>` актуальними. Викликати в `$effect.root`/компоненті. */
export function trackEffects(): () => void {
  const mq = window.matchMedia(REDUCED);
  const onMq = () => (sys.reduced = mq.matches);
  const onVis = () => (sys.hidden = document.hidden);
  mq.addEventListener("change", onMq);
  document.addEventListener("visibilitychange", onVis);
  const stop = $effect.root(() => {
    $effect(() =>
      applyRoot(document.documentElement, {
        effects: effectiveEffects(),
        palette: ui.settings.palette ?? "champagne",
        measuring: measuringNow(),
        hidden: sys.hidden,
      }),
    );
  });
  return () => {
    stop();
    mq.removeEventListener("change", onMq);
    document.removeEventListener("visibilitychange", onVis);
  };
}
