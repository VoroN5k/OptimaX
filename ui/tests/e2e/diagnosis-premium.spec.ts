import { expect, test, type Page } from "@playwright/test";
import { classicResult, comparison, gpuResult } from "./fixtures";
import { calls, emit, mockBackend } from "./mock";
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
    await mockBackend(page, { diag_state: { recording: false, last: classicResult } });
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
    // «Що робити далі» — справжні поради знахідок, кожна один раз.
    await expect(page.getByText("Що робити далі", { exact: true })).toBeVisible();
    await expect(page.locator("ul li").filter({ hasText: tk("diag.gpu_bound.advice") })).toHaveCount(1);
  });

  test("порада gpu_bound має «Перевірити A/B» і веде в «Порівняння» з грою й варіантами", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: classicResult } });
    await open(page);
    const row = page.locator("ul li").filter({ hasText: tk("diag.gpu_bound.advice") });
    await row.getByRole("button", { name: "Перевірити A/B" }).click();
    await expect(page.getByRole("tab", { name: tk("ui.exp.tab"), selected: true })).toBeVisible();
    await expect(page.locator("#exp-name-0")).toHaveValue("Як зараз");
    await expect(page.locator("#exp-name-1")).toHaveValue("Нижчі тіні, RT і відображення");
    await expect
      .poll(() => page.evaluate(() => Array.from(document.querySelectorAll("input")).some((i) => i.value === "re9.exe")))
      .toBe(true);
  });

  test("апскейл — кнопка в «Що робити далі»", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: classicResult } });
    await open(page);
    const row = page.locator("ul li").filter({ hasText: tk("diag.upscale.helps.advice") });
    await expect(row.getByRole("button", { name: tk("ui.up.open") })).toBeVisible();
  });

  test("порада не повторюється в картках знахідок; є підказка про повторний запис", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await open(page);
    await expect(page.getByText(tk("diag.gpu_bound.advice"))).toHaveCount(1);
    await expect(page.getByText(tk("diag.upscale.helps.advice"))).toHaveCount(1);
    await expect(
      page.getByText("Змінив щось? Запиши гру ще раз — результат порівняється з цим записом."),
    ).toBeVisible();
  });

  test("розбори — в закритій згортці «Для досвідчених»; плитки «Розбори» немає", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await open(page);
    await expect(page.getByRole("region", { name: tk("ui.diag.reviews") })).toHaveCount(0);
    await expect(page.locator("button.side")).toHaveCount(0);
    await expect(page.getByRole("button", { name: new RegExp(`^${tk("ui.tl.title")}`) })).toHaveCount(0);
    await page.getByRole("button", { name: /Для досвідчених/ }).click();
    await page.getByRole("button", { name: new RegExp(`^${tk("ui.tl.title")}`) }).click();
    await expect(page.getByRole("application", { name: tk("ui.tl.aria") })).toBeVisible();
  });

  test("A/B з «Історії» відкриває «Порівняння» з грою запису", async ({ page }) => {
    await mockBackend(page, { history_open: structuredClone(classicResult) });
    await open(page);
    await page.getByRole("tab", { name: tk("ui.history.tab") }).click();
    await page.getByRole("button", { name: tk("ui.history.open") }).first().click();
    const row = page.locator("ul li").filter({ hasText: tk("diag.gpu_bound.advice") });
    await row.getByRole("button", { name: "Перевірити A/B" }).click();
    await expect(page.getByRole("tab", { name: tk("ui.exp.tab"), selected: true })).toBeVisible();
    await expect(page.locator("#exp-name-1")).toHaveValue("Нижчі тіні, RT і відображення");
  });

  test("після history-saved — картка «Проти попереднього запису» з порогом і застереженням", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult }, history_vs_previous: comparison });
    await open(page);
    await expect(page.getByRole("region", { name: "Проти попереднього запису" })).toHaveCount(0);
    await emit(page, "history-saved", "id1");
    const card = page.getByRole("region", { name: "Проти попереднього запису" });
    await expect(card).toBeVisible();
    await expect(card).toContainText(tk("ui.history.trend.worse", { t: 8 }));
    await expect(card).toContainText("−13.6 %".replace("−", "-"));
    await expect(card).toContainText("Один запис проти одного; доказ — A/B.");
    expect((await calls(page)).find((c) => c.cmd === "history_vs_previous")?.args).toEqual({ id: "id1" });
  });

  test("перший запис гри — картки немає, головна картка на всю ширину", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult }, history_vs_previous: null });
    await open(page);
    await emit(page, "history-saved", "id1");
    await expect.poll(async () => (await calls(page)).some((c) => c.cmd === "history_vs_previous")).toBe(true);
    await expect(page.getByRole("region", { name: "Проти попереднього запису" })).toHaveCount(0);
  });

  test("новий запис (diag-done) скидає картку попереднього", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult }, history_vs_previous: comparison });
    await open(page);
    await emit(page, "history-saved", "id1");
    await expect(page.getByRole("region", { name: "Проти попереднього запису" })).toBeVisible();
    await emit(page, "diag-done", structuredClone(gpuResult));
    await expect(page.getByRole("region", { name: "Проти попереднього запису" })).toHaveCount(0);
  });

  test("інша сцена — попередження в картці", async ({ page }) => {
    const c = structuredClone(comparison);
    c.scene = "different";
    c.older.scene = "місто";
    c.newer.scene = "коридор";
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult }, history_vs_previous: c });
    await open(page);
    await emit(page, "history-saved", "id1");
    await expect(page.getByRole("region", { name: "Проти попереднього запису" })).toContainText(
      tk("ui.history.warn_scene", { a: "місто", b: "коридор" }),
    );
  });

  test("відкритий з «Історії» запис порівнюється з попереднім за своїм id", async ({ page }) => {
    await mockBackend(page, { history_vs_previous: comparison });
    await open(page);
    await page.getByRole("tab", { name: tk("ui.history.tab") }).click();
    await page.getByRole("button", { name: tk("ui.history.open") }).first().click();
    await expect(page.getByRole("region", { name: "Проти попереднього запису" })).toBeVisible();
    expect((await calls(page)).find((c) => c.cmd === "history_vs_previous")?.args).toEqual({
      id: comparison.newer.id,
    });
  });

  test("два фонові процеси (однаковий id знахідки) — результат рендериться, обидві поради видно", async ({ page }) => {
    const r = structuredClone(classicResult);
    const bg = (name: string) => ({
      finding: {
        id: "background",
        category: "background" as const,
        impact: "moderate" as const,
        confidence: 75,
        title: { key: "diag.background.title", args: { name, cpu: "13.0" } },
        detail: { key: "diag.background.detail", args: { name, cpu: "13.0" } },
        advice: { key: "diag.background.advice", args: { name, cpu: "13.0" } },
        evidence: [],
        next: null,
      },
      expected_gain: null,
    });
    r.diagnosis.verdicts.splice(1, 0, bg("chrome.exe"), bg("discord.exe"), bg("obs64.exe"));
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await mockBackend(page, { diag_state: { recording: false, last: r } });
    await open(page);
    for (const name of ["chrome.exe", "discord.exe", "obs64.exe"]) {
      // Кожна порада показана рівно один раз: угорі (перші три) або в картці.
      await page.getByRole("button", { name: tk("diag.background.title", { name, cpu: "13.0" }) }).click().catch(() => {});
      await expect(page.getByText(tk("diag.background.advice", { name, cpu: "13.0" }))).toHaveCount(1);
    }
    expect(errors).toEqual([]);
  });

  test("апскейл поза трійкою порад — кнопка лишається в картці знахідки", async ({ page }) => {
    const r = structuredClone(classicResult);
    const extra = (id: string, key: string) => ({
      finding: {
        id,
        category: "gpu" as const,
        impact: "info" as const,
        confidence: 70,
        title: { key: `${key}.title`, args: {} },
        detail: { key: `${key}.detail`, args: {} },
        advice: { key: `${key}.advice`, args: {} },
        evidence: [],
        next: null,
      },
      expected_gain: null,
    });
    // gpu_bound + 2 ще з порадою — upscale.helps (останній) випадає з трійки.
    r.diagnosis.verdicts.splice(2, 0, extra("vram.near_full", "diag.vram.near_full"), extra("present.composed", "diag.present.composed"));
    await mockBackend(page, { diag_state: { recording: false, last: r } });
    await open(page);
    await page.getByRole("button", { name: tk("diag.upscale.helps.title") }).click();
    await expect(page.getByRole("button", { name: tk("ui.up.open") })).toBeVisible();
  });

  test("відкритий з «Історії» запис — без підказки «запиши ще раз»", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await page.getByRole("tab", { name: tk("ui.history.tab") }).click();
    await page.getByRole("button", { name: tk("ui.history.open") }).first().click();
    await expect(page.getByText(tk("diag.summary.gpu.title", { game: "re9.exe" })).last()).toBeVisible();
    await expect(page.getByText(tk("ui.diag.next.rerecord"))).toHaveCount(0);
  });

  test("без чисел у доказах кільця немає — лише текст знахідки", async ({ page }) => {
    const r = structuredClone(classicResult);
    for (const v of r.diagnosis.verdicts) v.finding.evidence = [];
    await mockBackend(page, { diag_state: { recording: false, last: r } });
    await open(page);
    await expect(page.getByRole("heading", { name: tk("diag.summary.gpu.title", { game: "re9.exe" }) })).toBeVisible();
    await expect(page.getByRole("img", { name: /відсотків/ })).toHaveCount(0);
  });
});
