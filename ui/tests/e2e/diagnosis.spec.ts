import { expect, test, type Page } from "@playwright/test";
import { gpuResult } from "./fixtures";
import { calls, emit, mockBackend } from "./mock";
import { tk } from "./text";

async function openDiagnosis(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: tk("ui.nav.diagnosis") }).click();
  await expect(page.getByRole("heading", { level: 1, name: tk("ui.diag.title") })).toBeVisible();
}

test.describe("Діагностика", () => {
  test("результат: вердикт зверху, знахідки компактні, без позначки прикладу", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await openDiagnosis(page);
    await expect(page.getByText(tk("diag.summary.gpu.title", { game: "re9.exe" }))).toBeVisible();
    await expect(page.locator("[data-tour=diag-stats]")).toContainText("125.0");
    await expect(page.getByText(tk("ui.diag.findings_n", { n: 2 }))).toBeVisible();
    // Перша знахідка розгорнута (видно пораду), друга — лише заголовок.
    await expect(page.getByText(tk("diag.gpu_bound.advice"), { exact: false })).toBeVisible();
    await expect(page.getByText(tk("diag.upscale.helps.detail"))).toHaveCount(0);
    await page.getByRole("button", { name: tk("diag.upscale.helps.title") }).click();
    await expect(page.getByText(tk("diag.upscale.helps.detail"))).toBeVisible();
    await expect(page.getByText(tk("ui.tour.example_title"))).toHaveCount(0);
    // Налаштування запису згорнуті в рядок, тест роздільності — згорнутий блок.
    await expect(page.locator("[data-tour=diag-game]")).toHaveCount(0);
    await expect(page.getByRole("button", { name: tk("ui.res.title") })).toHaveAttribute("aria-expanded", "false");
  });

  test("запис: команда старту, хід і результат з подій бекенду", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await openDiagnosis(page);
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "diag_start")?.args)
      .toEqual({ exe: "re9.exe", seconds: 60, scene: null });
    await emit(page, "diag-progress", { phase: "recording", counted_s: 12, target_s: 60, elapsed_s: 14 });
    await expect(page.getByRole("button", { name: tk("ui.diag.stop") })).toBeVisible();
    const done = structuredClone(gpuResult);
    done.diagnosis.verdicts = done.diagnosis.verdicts.slice(0, 1);
    await emit(page, "diag-done", done);
    await expect(page.getByText(tk("ui.diag.no_more"))).toBeVisible();
  });

  test("мітка сцени передається в запис", async ({ page }) => {
    await mockBackend(page);
    await openDiagnosis(page);
    await page.getByLabel(tk("ui.diag.scene")).fill("  коридор ");
    await page.getByRole("button", { name: tk("ui.picker.refresh") }).waitFor();
    await page.locator("[data-tour=diag-game] input").fill("re9.exe");
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "diag_start")?.args)
      .toEqual({ exe: "re9.exe", seconds: 60, scene: "коридор" });
  });

  test("«Довідка» пояснює вердикти, Esc закриває", async ({ page }) => {
    await mockBackend(page);
    await openDiagnosis(page);
    await page.getByRole("button", { name: tk("ui.guide.open") }).click();
    const dialog = page.getByRole("dialog", { name: tk("ui.guide.title") });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(tk("ui.guide.v.cpu.title"));
    await expect(dialog).toContainText(tk("ui.guide.t.low1.text"));
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  });
  test("анатомія кадру: типовий кадр, черга і що розтягнуло ривки", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: structuredClone(gpuResult) } });
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.nav.diagnosis") }).click();
    const fold = page.getByRole("button", { name: new RegExp(tk("ui.anatomy.title")) });
    await expect(fold).toContainText(tk("ui.anatomy.summary", { ms: "8.0", n: 1 }));
    await fold.click();
    await expect(page.getByText(tk("ui.anatomy.queued", { pct: 34 }))).toBeVisible();
    await expect(page.getByText(`${tk("ui.anatomy.stall.cpu_busy")} · 1`)).toBeVisible();
    await expect(page.locator("table.spikes tbody tr")).toHaveCount(1);
    await expect(page.locator("table.spikes tbody tr")).toContainText("55.2");
  });
});
