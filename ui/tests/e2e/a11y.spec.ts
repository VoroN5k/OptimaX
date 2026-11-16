import { expect, test, type Page } from "@playwright/test";
import { gpuResult } from "./fixtures";
import { mockBackend } from "./mock";
import { go } from "./nav";

// Доступність (docs/DESIGN.md розд. 10, CLAUDE.md 10.6): контраст токенів, видимий фокус,
// доступні імена й розмір цілей на всіх розділах.
const SCREENS = [
  "analysis",
  "hardware",
  "laptop",
  "network",
  "diagnosis",
  "stability",
  "optimization",
  "games",
  "cleanup",
  "startup",
  "backups",
  "settings",
];

/** Контраст WCAG між кольором тексту й фоном (обидва — CSS-кольори; прозорі накладаються на `under`). */
async function contrasts(page: Page, pairs: [string, string][]): Promise<{ pair: string; ratio: number }[]> {
  return page.evaluate((pairs) => {
    // Розбір кольору через canvas: fillStyle нормалізує будь-який CSS-колір до #rrggbb чи rgba().
    const ctx = document.createElement("canvas").getContext("2d");
    const rgba = (v: string): [number, number, number, number] => {
      if (!ctx) return [0, 0, 0, 1];
      ctx.fillStyle = "#000";
      ctx.fillStyle = v;
      const s = String(ctx.fillStyle);
      if (s.startsWith("#")) {
        const n = parseInt(s.slice(1), 16);
        return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 1];
      }
      const m = s.match(/[\d.]+/g)?.map(Number) ?? [0, 0, 0, 1];
      return [m[0] ?? 0, m[1] ?? 0, m[2] ?? 0, m[3] ?? 1];
    };
    const over = (top: [number, number, number, number], under: [number, number, number]): [number, number, number] => [
      top[0] * top[3] + under[0] * (1 - top[3]),
      top[1] * top[3] + under[1] * (1 - top[3]),
      top[2] * top[3] + under[2] * (1 - top[3]),
    ];
    const lum = (c: [number, number, number]) => {
      const f = (v: number) => {
        const s = v / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
      };
      return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
    };
    const css = getComputedStyle(document.documentElement);
    const val = (t: string) => (t.startsWith("--") ? css.getPropertyValue(t).trim() : t);
    const base = rgba(val("--bg-base"));
    const out = pairs.map(([fg, bg]) => {
      const b = over(rgba(val(bg)), [base[0], base[1], base[2]]);
      const f = over(rgba(val(fg)), b);
      const [l1, l2] = [lum(f), lum(b)].sort((x, y) => y - x) as [number, number];
      return { pair: `${fg} на ${bg}`, ratio: (l1 + 0.05) / (l2 + 0.05) };
    });
    return out;
  }, pairs);
}

test.describe("Доступність", () => {
  test("контраст: текст і семантичні кольори ≥ 4.5:1 на фоні, плитці й головній картці", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    // Плитка й головна картка — градієнти з прозорістю; беремо темніший і світліший кінці.
    const surfaces = ["--bg-base", "--tile-solid", "rgba(19,21,28,.82)", "rgba(10,11,16,.82)", "rgba(20,22,29,.92)"];
    const texts = ["--text", "--text-2", "--text-3", "--a1", "--a2", "--warn", "--ok", "--bad", "--gpu", "--queue"];
    const pairs: [string, string][] = [];
    for (const t of texts) for (const s of surfaces) pairs.push([t, s]);
    // Текст на акцентній кнопці (обидва кінці градієнта).
    pairs.push(["--onA", "--a1"], ["--onA", "--a2"]);
    const res = await contrasts(page, pairs);
    // Перевірка самого розбору: білий текст на майже чорному — понад 15:1.
    expect(res[0]?.ratio ?? 0).toBeGreaterThan(15);
    const weak = res.filter((r) => r.ratio < 4.5).map((r) => `${r.pair}: ${r.ratio.toFixed(2)}`);
    expect(weak).toEqual([]);
  });

  test("усі розділи: у кнопок і полів є доступне ім'я, цілі ≥ 32 px заввишки", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await page.goto("/");
    const problems: string[] = [];
    for (const s of SCREENS) {
      await go(page, s);
      await page.waitForTimeout(150);
      const found = await page.evaluate(() => {
        const out: string[] = [];
        const els = document.querySelectorAll<HTMLElement>(
          "button, a[href], input:not([type=hidden]), select, textarea, [role=tab], [role=radio], [role=switch]",
        );
        for (const el of els) {
          const r = el.getBoundingClientRect();
          if (r.width === 0 || r.height === 0 || el.closest("[aria-hidden=true]")) continue;
          const id = el.id;
          const labelled =
            (el.getAttribute("aria-label") ?? "").trim() ||
            (el.getAttribute("aria-labelledby") ?? "").trim() ||
            (el.getAttribute("title") ?? "").trim() ||
            (el.textContent ?? "").trim() ||
            (el as HTMLInputElement).placeholder ||
            (id && document.querySelector(`label[for="${id}"]`)) ||
            el.closest("label");
          const what = `${el.tagName.toLowerCase()}${el.className ? "." + String(el.className).split(" ")[0] : ""} «${(el.textContent ?? "").trim().slice(0, 30)}»`;
          if (!labelled) out.push(`без імені: ${what}`);
          const inline = el.tagName === "A" && getComputedStyle(el).display === "inline";
          const box = el.getAttribute("type") === "checkbox" || el.getAttribute("type") === "radio";
          if (!inline && !box && r.height < 32) out.push(`${Math.round(r.height)} px: ${what}`);
        }
        return out;
      });
      for (const f of found) problems.push(`${s}: ${f}`);
    }
    expect([...new Set(problems)]).toEqual([]);
  });

  test("фокус клавіатури видно на кожному елементі (перші 40 Tab на кожному розділі)", async ({ page }) => {
    test.setTimeout(90_000);
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await page.goto("/");
    const problems: string[] = [];
    for (const s of SCREENS) {
      await go(page, s);
      await page.locator("h1").first().click();
      for (let i = 0; i < 40; i++) {
        await page.keyboard.press("Tab");
        const r = await page.evaluate(() => {
          const el = document.activeElement as HTMLElement | null;
          if (!el || el === document.body) return null;
          const cs = getComputedStyle(el);
          const ring = (cs.outlineStyle !== "none" && parseFloat(cs.outlineWidth) >= 2) || cs.boxShadow !== "none";
          return ring ? null : `${el.tagName.toLowerCase()}.${String(el.className).split(" ")[0]} «${(el.textContent ?? "").trim().slice(0, 30)}»`;
        });
        if (r) problems.push(`${s}: ${r}`);
      }
    }
    expect([...new Set(problems)]).toEqual([]);
  });
});
