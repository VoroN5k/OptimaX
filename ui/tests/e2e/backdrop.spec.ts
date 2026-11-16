import { expect, test, type Page } from "@playwright/test";
import { buildBackdropData, IDLE, RUN } from "../../src/lib/backdrop";
import { gpuResult, settings } from "./fixtures";
import { mockBackend } from "./mock";
import { go } from "./nav";

// Фоновий шар (docs/DESIGN.md розд. 9, 11): дані лише з запису, підписи лише для реальних даних,
// вимикається поза «Повними» ефектами й під час вимірювань.
const view = gpuResult.diagnosis.view;

test.describe("Фон: дані з запису", () => {
  test("доріжка головного потоку: точні проміжки біля ривка, решта — частки кошиків", () => {
    const d = buildBackdropData(view);
    expect(d).not.toBeNull();
    if (!d || !view?.main) return;
    expect(d.lanes.map((l) => l.key)).toEqual(["main"]);
    const L = d.lanes[0];
    if (!L) return;
    // Відсортовано за початком, без накладань.
    for (let i = 1; i < L.start.length; i++) {
      expect(L.start[i] ?? 0).toBeGreaterThanOrEqual((L.start[i - 1] ?? 0) + (L.dur[i - 1] ?? 0) - 0.5);
    }
    // Точний проміжок «диск» (стан 3) 12522+36 мс → «не працював».
    const i = Array.from(L.start).findIndex((s) => Math.abs(s - 12522) < 0.01);
    expect(i).toBeGreaterThanOrEqual(0);
    expect(L.state[i]).toBe(IDLE);
    expect(L.dur[i]).toBeCloseTo(36);
    // Кошик 0 (1000–1250 мс): 180 з 250 мс працював → перший відрізок 180 мс «працював».
    expect(L.start[0]).toBe(1000);
    expect(L.state[0]).toBe(RUN);
    expect(L.dur[0]).toBeCloseTo(180);
  });

  test("ривки — лише ті, що визначив бекенд", () => {
    const d = buildBackdropData(view);
    if (!d) throw new Error("немає даних");
    const spikes = Array.from(d.frameSpike).reduce((a, b) => a + b, 0);
    expect(spikes).toBe(1);
    const k = d.frameSpike.indexOf(1);
    expect(d.frameEnd[k]).toBe(12564);
    expect(d.frameFt[k]).toBe(60);
  });

  test("без станів потоків чи кадрів — даних немає (декоративний фон)", () => {
    expect(buildBackdropData(null)).toBeNull();
    if (!view) return;
    expect(buildBackdropData({ ...view, main: null, render: null })).toBeNull();
  });
});

async function labelsDrawn(page: Page): Promise<boolean> {
  return page.locator(".backdrop canvas.labels").evaluate((c: HTMLCanvasElement) => {
    const ctx = c.getContext("2d");
    if (!ctx) return false;
    const px = ctx.getImageData(0, 0, c.width, c.height).data;
    for (let i = 3; i < px.length; i += 4) if ((px[i] ?? 0) > 0) return true;
    return false;
  });
}

test.describe("Фон: коли показується", () => {
  test.beforeEach(async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "no-preference" });
  });

  test("без запису — декоративний, без підписів", async ({ page }) => {
    await mockBackend(page, { get_settings: { ...settings, effects: "full" } });
    await page.goto("/");
    const bd = page.locator(".backdrop");
    await expect(bd).toBeVisible();
    expect(await labelsDrawn(page)).toBe(false);
  });

  test("з записом — підписи й легенда; «Декоративний» їх прибирає", async ({ page }) => {
    await mockBackend(page, {
      get_settings: { ...settings, effects: "full" },
      diag_state: { recording: false, last: gpuResult },
    });
    await page.goto("/");
    await expect.poll(() => labelsDrawn(page)).toBe(true);
    await go(page, "settings");
    await page.getByRole("radio", { name: "Декоративний" }).click();
    await expect.poll(() => labelsDrawn(page)).toBe(false);
  });

  test("вимкнений у «Спокійних», під час запису й налаштуванням «Вимкнений»", async ({ page }) => {
    await mockBackend(page, {
      get_settings: { ...settings, effects: "calm" },
      diag_state: { recording: true, last: null },
    });
    await page.goto("/");
    await expect(page.locator(".backdrop")).toBeHidden();
    await go(page, "settings");
    await page.getByRole("radio", { name: "Повні" }).click();
    await expect(page.locator(".backdrop")).toBeVisible();
    await page.getByRole("radio", { name: "Вимкнений" }).click();
    await expect(page.locator(".backdrop")).toBeHidden();
    await page.getByRole("radio", { name: "Реальні дані" }).click();
    // Запис іде (стан із бекенду) — фон вимикається.
    await go(page, "diagnosis");
    await expect(page.locator("html")).toHaveClass(/\bmeasuring\b/);
    await expect(page.locator(".backdrop")).toBeHidden();
  });
});
