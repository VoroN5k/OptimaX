// Числа для кільця головної знахідки (docs/DESIGN.md розд. 11): лише з реальних доказів
// діагностики — значень `value.pct` / `value.pct_needed` з числом у `v`. Нічого не обчислюємо
// й не вигадуємо: якщо потрібного доказу немає, кільця (чи його половини) немає.
import type { Bottleneck } from "./bindings/Bottleneck";
import type { Evidence } from "./bindings/Evidence";
import type { Finding } from "./bindings/Finding";

/** Одна дуга кільця: відсоток і ключ доказу (підпис береться з самого доказу). */
export type RingPart = { value: number; evidence: Evidence };

/** Що показати в кільці: зовнішня дуга — головне обмеження, внутрішня — для порівняння. */
export type FindingRing = { outer: RingPart; inner: RingPart | null; caption: string };

function percent(e: Evidence): number | null {
  if (e.value.key !== "value.pct" && e.value.key !== "value.pct_needed") return null;
  const n = Number(e.value.args?.v);
  return Number.isFinite(n) ? n : null;
}

function part(fs: Finding[], keys: string[]): RingPart | null {
  for (const k of keys) {
    for (const f of fs) {
      const e = f.evidence.find((x) => x.label.key === k);
      const v = e ? percent(e) : null;
      if (e && v !== null) return { value: v, evidence: e };
    }
  }
  return null;
}

/** Кільце для запису з таким вузьким місцем. Підсумкова знахідка має лише FPS, тож числа шукаємо в
 *  усіх знахідках запису (правило-вердикт `gpu_bound` / `cpu_bound` їх містить). `null` — чисел немає. */
export function findingRing(f: Finding[], bottleneck: Bottleneck): FindingRing | null {
  if (bottleneck === "cpu") {
    const main = part(f, ["diag.ev.main_thread_busy"]);
    const outer = main ?? part(f, ["diag.ev.core_saturated_share"]);
    if (!outer) return null;
    return {
      outer,
      inner: part(f, ["diag.ev.gpu_avg"]),
      caption: main ? "ui.diag.ring.main_thread" : "ui.diag.ring.core",
    };
  }
  if (bottleneck === "gpu") {
    const outer = part(f, ["diag.ev.gpu_busy_share", "diag.ev.gpu_avg"]);
    if (!outer) return null;
    const avg = outer.evidence.label.key === "diag.ev.gpu_avg" ? null : part(f, ["diag.ev.gpu_avg"]);
    return { outer, inner: avg, caption: "ui.diag.ring.gpu" };
  }
  return null;
}

/** Заголовок знахідки для головної картки, текст не змінюється — лише ділиться для показу.
 *  «А — Б»: «А» світле, «Б» акцентом («re9.exe: вузьке місце — відеокарта»). Інакше до першої
 *  двокрапки: перше слово світле, решта акцентом, хвіст після двокрапки — окремим рядком. */
export function splitTitle(title: string): { light: string; strong: string; tail: string | null } {
  const dash = title.lastIndexOf(" — ");
  if (dash > 0) return { light: title.slice(0, dash), strong: title.slice(dash + 3), tail: null };
  const i = title.indexOf(": ");
  const head = i > 0 ? title.slice(0, i) : title;
  const tail = i > 0 ? title.slice(i + 2) : null;
  const sp = head.indexOf(" ");
  return sp > 0 ? { light: head.slice(0, sp), strong: head.slice(sp + 1), tail } : { light: "", strong: head, tail };
}
