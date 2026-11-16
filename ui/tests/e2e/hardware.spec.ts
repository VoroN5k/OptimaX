import { expect, test, type Page } from "@playwright/test";
import { hwSnap } from "./hardwareFixture";
import { emit, mockBackend } from "./mock";
import { go } from "./nav";
import { tk } from "./text";

// «Залізо»: лише виміряне; ядро в стелі, обмеження частоти й частоти нижче номіналу — текстом.
async function open(page: Page) {
  await mockBackend(page);
  await page.goto("/");
  await go(page, "hardware");
  await emit(page, "metrics", hwSnap);
}

test.describe("Залізо", () => {
  test("смуга живих показників і найзавантаженіше ядро", async ({ page }) => {
    await open(page);
    const strip = page.getByRole("region", { name: tk("ui.hw.kpi") });
    await expect(strip).toContainText("31%");
    await expect(strip).toContainText("97%");
    await expect(strip).toContainText("18.6 ГБ / 32.0 ГБ");
    await expect(page.getByRole("listitem", { name: `${tk("ui.hw.core", { n: 0 })}: 97%` })).toBeVisible();
  });

  test("до першого заміру — «—» і пояснення", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await go(page, "hardware");
    await expect(page.getByRole("region", { name: tk("ui.hw.kpi") }).getByText("—")).toHaveCount(4);
    await expect(page.getByText(tk("ui.hw.waiting"))).toBeVisible();
  });

  test("обмеження частоти — словами, не кодом NVML", async ({ page }) => {
    await open(page);
    await expect(page.getByText(tk("ui.hw.thr.sw_power_cap"))).toBeVisible();
    await expect(page.getByText("sw_power_cap")).toHaveCount(0);
  });

  test("пам'ять нижче номіналу й монітор нижче максимуму позначені текстом", async ({ page }) => {
    await open(page);
    await expect(page.getByText("2133 / 3600 MT/s")).toBeVisible();
    await expect(page.getByText(tk("ui.hw.below_rated"))).toHaveCount(1);
    await expect(page.getByText(tk("ui.hw.hz_below"))).toHaveCount(1);
  });
});
