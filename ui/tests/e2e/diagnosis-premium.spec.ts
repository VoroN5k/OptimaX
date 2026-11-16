import { expect, test, type Page } from "@playwright/test";
import { gpuResult } from "./fixtures";
import { mockBackend } from "./mock";
import { go } from "./nav";
import { tk } from "./text";

// Екран «Діагностика» в стилі «Pro Premium» (docs/DESIGN.md розд. 6): лише реальні числа.
async function open(page: Page) {
  await page.goto("/");
  await go(page, "diagnosis");
}

test.describe("Діагностика: Pro Premium", () => {
  test("порожній стан: показники «—» і пояснення, кільця немає", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    const strip = page.getByRole("region", { name: tk("ui.diag.empty_kpi") });
    await expect(strip.getByText("—")).toHaveCount(4);
    await expect(page.getByText(tk("ui.diag.empty_title"))).toBeVisible();
    await expect(page.getByRole("img", { name: /відсотків/ })).toHaveCount(0);
  });

  test("результат: смуга показників, головна знахідка, кільце з доказів, розбори", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await open(page);
    const strip = page.getByRole("region", { name: tk("ui.diag.kpi.label") });
    await expect(strip).toContainText("125.0");
    await expect(strip).toContainText("10.99");
    await expect(strip).toContainText(tk("ui.diag.confidence", { v: 81 }));
    // Кільце — з доказів gpu_bound (97 % частки й 98 % середнього), не вигадане.
    const ring = page.getByRole("img", {
      name: `${tk("ui.diag.ring.aria_part", { label: tk("diag.ev.gpu_busy_share"), v: 97 })}; ${tk("ui.diag.ring.aria_part", { label: tk("diag.ev.gpu_avg"), v: 98 })}`,
    });
    await expect(ring).toBeVisible();
    // «Що можна перевірити» — справжні поради знахідок.
    await expect(page.getByText(tk("ui.diag.check"))).toBeVisible();
    await expect(page.locator("ul li").filter({ hasText: tk("diag.gpu_bound.advice") })).toHaveCount(1);
    // Бічна плитка — друга знахідка; розбори відкривають свої блоки.
    await expect(page.locator("button.side")).toContainText(tk("diag.gpu_bound.title", { share: "97" }));
    const reviews = page.getByRole("region", { name: tk("ui.diag.reviews") });
    await reviews.getByRole("button", { name: new RegExp(tk("ui.tl.title")) }).click();
    await expect(page.getByRole("application", { name: tk("ui.tl.aria") })).toBeVisible();
  });

  test("без чисел у доказах кільця немає — лише текст знахідки", async ({ page }) => {
    const r = structuredClone(gpuResult);
    for (const v of r.diagnosis.verdicts) v.finding.evidence = [];
    await mockBackend(page, { diag_state: { recording: false, last: r } });
    await open(page);
    await expect(page.getByRole("heading", { name: tk("diag.summary.gpu.title", { game: "re9.exe" }) })).toBeVisible();
    await expect(page.getByRole("img", { name: /відсотків/ })).toHaveCount(0);
  });
});
