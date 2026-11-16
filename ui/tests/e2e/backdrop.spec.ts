import { expect, test, type Page } from "@playwright/test";
import { height } from "../../src/lib/backdrop";
import { gpuResult, settings } from "./fixtures";
import { mockBackend } from "./mock";
import { go } from "./nav";

// Фоновий шар «Шовк» (docs/DESIGN.md розд. 9): суто декоративний, вимикається поза «Повними»
// ефектами, налаштуванням і під час вимірювань; останній запис на нього не впливає.

test.describe("Фон: поверхня", () => {
  test("висота обмежена й плавна", () => {
    let max = 0;
    let step = 0;
    for (let t = 0; t < 600; t += 7.3) {
      for (let x = -15; x <= 15; x += 0.5) {
        for (let z = -7; z <= 7; z += 0.5) {
          const y = height(x, z, t);
          max = Math.max(max, Math.abs(y));
          // Сусідні точки сітки (крок ≈ 0.32) не стрибають — складки пологі.
          step = Math.max(step, Math.abs(height(x + 0.32, z, t) - y));
        }
      }
    }
    expect(max).toBeLessThanOrEqual(2.15);
    expect(step).toBeLessThan(0.45);
  });
});

async function drawn(page: Page): Promise<boolean> {
  return page.locator(".backdrop canvas").evaluate((c: HTMLCanvasElement) => {
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

  test("з «Повними» ефектами малюється, з записом і без — однаково декоративний", async ({ page }) => {
    await mockBackend(page, {
      get_settings: { ...settings, effects: "full" },
      diag_state: { recording: false, last: gpuResult },
    });
    await page.goto("/");
    await expect(page.locator(".backdrop")).toBeVisible();
    await expect.poll(() => drawn(page)).toBe(true);
    await expect(page.locator(".backdrop canvas")).toHaveCount(1);
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
    await page.getByRole("radio", { name: "Увімкнений" }).click();
    // Запис іде (стан із бекенду) — фон вимикається.
    await go(page, "diagnosis");
    await expect(page.locator("html")).toHaveClass(/\bmeasuring\b/);
    await expect(page.locator(".backdrop")).toBeHidden();
  });
});
