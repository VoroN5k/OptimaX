import { expect, test, type Page } from "@playwright/test";
import type { CheckResult } from "../../src/lib/bindings/CheckResult";
import type { DiagnosisResult } from "../../src/lib/bindings/DiagnosisResult";
import { comparison, gpuResult } from "./fixtures";
import { calls, emit, mockBackend } from "./mock";
import { go } from "./nav";
import { tk } from "./text";

// «Перевірити мою гру» (docs/superpowers/specs/2026-11-19-check-my-game-design.md): «Огляд» запускає,
// «Діагностика» веде кроками; повторний запис порівнюється саме з першим («було»).

/** Результат «є що покращити»: головна дія — порада gpu_bound (A/B). */
function fixable(): DiagnosisResult {
  const r = structuredClone(gpuResult);
  if (!r.diagnosis.story) throw new Error("story");
  r.diagnosis.story.outcome = "fixable";
  r.diagnosis.story.action = 1;
  r.diagnosis.story.others = [2];
  return r;
}

/** Fine з двома фоновими процесами з тим самим id (різняться порадою). */
function twoBackground(): DiagnosisResult {
  const r = structuredClone(gpuResult);
  const bg = (name: string) => ({
    finding: {
      id: "background",
      category: "background" as const,
      impact: "info" as const,
      confidence: 80,
      title: { key: "diag.background.title", args: { name, cpu: "4" } },
      detail: { key: "diag.background.detail", args: { name, cpu: "4" } },
      advice: { key: "diag.background.advice", args: { name } },
      evidence: [],
      next: null,
    },
    expected_gain: null,
  });
  r.diagnosis.verdicts = [r.diagnosis.verdicts[0], bg("chrome.exe"), bg("discord.exe")];
  if (r.diagnosis.story) r.diagnosis.story.others = [1, 2];
  return r;
}

function checkRes(over: Partial<CheckResult> = {}): CheckResult {
  return {
    comparison: { ...structuredClone(comparison), trend: "same" },
    focus: "lows",
    trend: "better",
    change_pct: 27.1,
    ab: null,
    advice: null,
    ...over,
  };
}

const steps = (page: Page) => page.getByRole("list", { name: tk("ui.check.steps.label") });
const stepsBar = (page: Page) => page.getByRole("region", { name: tk("ui.check.title") });

/** «Огляд» → «Почати» → «Діагностика» в режимі сценарію. */
async function startScenario(page: Page) {
  await page.goto("/");
  await go(page, "analysis");
  await page.getByRole("button", { name: tk("ui.check.start") }).click();
  await expect(steps(page)).toBeVisible();
}

/** Перший запис сценарію: старт, результат, збереження в історії як `id`. */
async function recordFirst(page: Page, r: DiagnosisResult, id: string | null = "base1") {
  await page.getByRole("button", { name: tk("ui.picker.refresh") }).waitFor();
  await page.locator("[data-tour=diag-game] input").fill("re9.exe");
  await page.getByRole("button", { name: tk("ui.diag.start") }).click();
  await expect.poll(async () => (await calls(page)).some((c) => c.cmd === "diag_start")).toBe(true);
  await emit(page, "diag-done", r);
  if (id) await emit(page, "history-saved", id);
}

const starts = async (page: Page) =>
  (await calls(page)).filter((c) => c.cmd === "diag_start").map((c) => c.args);

test.describe("Перевірити мою гру", () => {
  test("картка «Огляду» веде в «Діагностику» зі смужкою кроків і 120 с", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await startScenario(page);
    await expect(page.getByRole("tab", { name: tk("ui.crash.tab_lag"), selected: true })).toBeVisible();
    await expect(steps(page).locator("[aria-current=step]")).toContainText(tk("ui.check.steps.game"));
    await expect(page.getByRole("radio", { name: "120 с" })).toBeChecked();
    // Старий результат не видається за перший крок сценарію.
    await expect(page.getByRole("region", { name: tk("ui.story.label") })).toHaveCount(0);
  });

  test("остання перевірка на «Огляді»", async ({ page }) => {
    await mockBackend(page, { check_last: checkRes() });
    await page.goto("/");
    await go(page, "analysis");
    const c = checkRes().comparison;
    await expect(
      page.getByText(
        tk("ui.check.last", {
          game: c.newer.game,
          metric: tk("ui.check.metric.lows"),
          from: (c.older.low_1pct_fps ?? 0).toFixed(0),
          to: (c.newer.low_1pct_fps ?? 0).toFixed(0),
          verdict: tk("ui.check.verdict.better"),
        }),
      ),
    ).toBeVisible();
  });

  test("«Я зробив» передає check_of, гру, тривалість і сцену; ланцюжок — з першим записом", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await page.getByLabel(tk("ui.diag.scene")).fill("ринок");
    await recordFirst(page, fixable());
    await expect(steps(page).locator("[aria-current=step]")).toContainText(tk("ui.check.steps.action"));
    const block = page.getByRole("region", { name: tk("ui.check.do_title") });
    await expect(block).toContainText(tk("ui.check.same_scene"));
    await page.getByRole("button", { name: tk("ui.check.recheck") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(2);
    expect((await starts(page))[1]).toEqual({ exe: "re9.exe", seconds: 120, scene: "ринок", checkOf: "base1" });
    // «Стало» збережено під іншим id — «було» лишається першим.
    await emit(page, "diag-done", fixable());
    await emit(page, "history-saved", "after1");
    await expect(steps(page).locator("[aria-current=step]")).toContainText(tk("ui.check.steps.result"));
    await page.getByRole("button", { name: tk("ui.check.recheck") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(3);
    expect((await starts(page))[2]).toMatchObject({ checkOf: "base1" });
  });

  test("«Усе гаразд» без порад — завершальна картка, «Перевірити іншу гру»", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    const r = structuredClone(gpuResult);
    if (r.diagnosis.story) r.diagnosis.story.others = [];
    await recordFirst(page, r);
    const done = page.getByRole("region", { name: tk("ui.check.done_title") });
    await expect(done).toContainText(tk("ui.check.done_text"));
    await expect(page.getByRole("button", { name: tk("ui.check.recheck") })).toHaveCount(0);
    await expect(page.getByRole("button", { name: tk("ui.story.try") })).toHaveCount(0);
    await done.getByRole("button", { name: tk("ui.check.another") }).click();
    await expect(steps(page).locator("[aria-current=step]")).toContainText(tk("ui.check.steps.game"));
  });

  test("«Спробувати» варіант після «Усе гаразд» → «Я зробив» шле checkFinding", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, gpuResult);
    const story = page.getByRole("region", { name: tk("ui.story.label") });
    await expect(steps(page).locator("[aria-current=step]")).toContainText(tk("ui.check.steps.verdict"));
    const row = story.locator("li").filter({ hasText: tk("diag.upscale.helps.advice") });
    await row.getByRole("button", { name: tk("ui.story.try") }).click();
    await expect(row).toContainText(tk("ui.story.tried"));
    await expect(steps(page).locator("[aria-current=step]")).toContainText(tk("ui.check.steps.action"));
    const block = page.getByRole("region", { name: tk("ui.check.do_title") });
    await expect(block).toContainText(tk("diag.upscale.helps.advice"));
    await page.getByRole("button", { name: tk("ui.check.recheck") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(2);
    expect((await starts(page))[1]).toEqual({ exe: "re9.exe", seconds: 120, scene: null, checkOf: "base1", checkFinding: { id: "upscale.helps", advice: { key: "diag.upscale.helps.advice", args: {} } } });
  });

  test("«Спробувати» інший варіант при головній дії; без вибору — головна дія", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, fixable());
    const block = page.getByRole("region", { name: tk("ui.check.do_title") });
    await expect(block).toContainText(tk("diag.gpu_bound.advice"));
    const story = page.getByRole("region", { name: tk("ui.story.label") });
    await story.getByText(tk("ui.story.others"), { exact: false }).click();
    await story.locator("li").filter({ hasText: tk("diag.upscale.helps.advice") }).getByRole("button", { name: tk("ui.story.try") }).click();
    await expect(block).toContainText(tk("diag.upscale.helps.advice"));
    await page.getByRole("button", { name: tk("ui.check.recheck") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(2);
    expect((await starts(page))[1]).toMatchObject({ checkOf: "base1", checkFinding: { id: "upscale.helps", advice: { key: "diag.upscale.helps.advice", args: {} } } });
  });

  test("замало даних — «Записати ще раз», без «Я зробив»", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    const r = fixable();
    if (r.diagnosis.story) r.diagnosis.story.outcome = "insufficient";
    await recordFirst(page, r, null);
    await expect(page.getByRole("button", { name: tk("ui.check.again") })).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.check.recheck") })).toHaveCount(0);
  });

  test("«Було / стало»: краще за 1% low — «Залиш так», без дубля «Проти попереднього»", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: fixable() }, check_result: checkRes(), history_vs_previous: comparison });
    await page.goto("/");
    await go(page, "diagnosis");
    await emit(page, "history-saved", "after1");
    const card = page.getByRole("region", { name: tk("ui.check.card") });
    await expect(card).toContainText(tk("ui.check.keep"));
    await expect(card).toContainText("27.1");
    await expect(card).not.toContainText(tk("ui.history.trend.same", { t: 8 }));
    await expect(page.getByRole("region", { name: tk("ui.prev.title") })).toHaveCount(0);
  });

  test("«Було / стало»: гірше — «Поверни як було»", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: fixable() }, check_result: checkRes({ trend: "worse", focus: "fps", change_pct: -12 }) });
    await page.goto("/");
    await go(page, "diagnosis");
    await emit(page, "history-saved", "after1");
    await expect(page.getByRole("region", { name: tk("ui.check.card") })).toContainText(tk("ui.check.revert"));
  });

  test("«Було / стало»: у межах похибки — A/B відкриває «Порівняння» з варіантами", async ({ page }) => {
    const ab: CheckResult["ab"] = [
      { key: "ui.exp.preset.reflex_off", args: {} },
      { key: "ui.exp.preset.reflex_on", args: {} },
    ];
    await mockBackend(page, { diag_state: { recording: false, last: fixable() }, check_result: checkRes({ trend: "same", focus: "fps", change_pct: 2, ab }) });
    await page.goto("/");
    await go(page, "diagnosis");
    await emit(page, "history-saved", "after1");
    const card = page.getByRole("region", { name: tk("ui.check.card") });
    await card.getByRole("button", { name: tk("ui.check.ab") }).click();
    await expect(page.getByRole("tab", { name: tk("ui.exp.tab"), selected: true })).toBeVisible();
    await expect(page.locator("#exp-name-1")).toHaveValue(tk("ui.exp.preset.reflex_on"));
  });

  test("без сценарію «Діагностика» без смужки кроків", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await page.goto("/");
    await go(page, "diagnosis");
    await expect(page.getByRole("region", { name: tk("ui.story.label") })).toBeVisible();
    await expect(stepsBar(page)).toHaveCount(0);
  });

  test("«Завершити перевірку» прибирає смужку", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await page.getByRole("button", { name: tk("ui.check.finish") }).click();
    await expect(stepsBar(page)).toHaveCount(0);
  });

  test("запис-перевірка з «Історії» показує «Було / стало»", async ({ page }) => {
    await mockBackend(page, { check_result: checkRes() });
    await page.goto("/");
    await go(page, "diagnosis");
    await page.getByRole("tab", { name: tk("ui.history.tab") }).click();
    await page.getByRole("button", { name: tk("ui.history.open") }).first().click();
    await expect(page.getByRole("region", { name: tk("ui.check.card") })).toContainText(tk("ui.check.keep"));
    await expect(page.getByRole("button", { name: tk("ui.story.try") })).toHaveCount(0);
  });

  test("«Було / стало» показує, що змінено", async ({ page }) => {
    await mockBackend(page, {
      diag_state: { recording: false, last: fixable() },
      check_result: checkRes({ advice: { key: "diag.upscale.helps.advice", args: {} } }),
    });
    await page.goto("/");
    await go(page, "diagnosis");
    await emit(page, "history-saved", "after1");
    const card = page.getByRole("region", { name: tk("ui.check.card") });
    await expect(card).toContainText(tk("ui.check.changed"));
    await expect(card).toContainText(tk("diag.upscale.helps.advice"));
  });

  test("нижня кнопка після «було» теж записує перевірку", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, fixable());
    // Звичайної кнопки «Почати запис» у сценарії з «було» немає — її місце займає «Я зробив».
    await expect(page.getByRole("button", { name: tk("ui.diag.start") })).toHaveCount(0);
    await page.getByRole("button", { name: tk("ui.check.recheck") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(2);
    expect((await starts(page))[1]).toMatchObject({ checkOf: "base1" });
  });

  test("history-saved раніше за diag-done — однаково «Я зробив»", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await page.getByRole("button", { name: tk("ui.picker.refresh") }).waitFor();
    await page.locator("[data-tour=diag-game] input").fill("re9.exe");
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(1);
    await emit(page, "history-saved", "base1");
    await emit(page, "diag-done", fixable());
    await expect(page.getByRole("button", { name: tk("ui.check.recheck") })).toBeVisible();
  });

  test("звичайний запис у сценарії не видається за «Було / стало»", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, gpuResult);
    // «Усе гаразд» — запис ще раз звичайний; крок лишається «Вердикт».
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(2);
    await emit(page, "diag-done", gpuResult);
    await emit(page, "history-saved", "other1");
    await expect(steps(page).locator("[aria-current=step]")).toContainText(tk("ui.check.steps.verdict"));
  });

  test("інша гра або інша вкладка завершують сценарій", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, fixable());
    await page.getByRole("button", { name: tk("ui.diag.setup") }).first().click();
    await page.locator("[data-tour=diag-game] input").fill("cs2.exe");
    await expect(stepsBar(page)).toHaveCount(0);
    await startScenario(page);
    await page.getByRole("tab", { name: tk("ui.history.tab") }).click();
    await page.getByRole("tab", { name: tk("ui.crash.tab_lag") }).click();
    await expect(stepsBar(page)).toHaveCount(0);
  });

  test("«було» недоступне — зрозумілий текст і «Почати заново»", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, fixable());
    await page.getByRole("button", { name: tk("ui.check.recheck") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(2);
    await emit(page, "diag-error", { kind: "not_found", message: "повний запис уже не зберігається" });
    await expect(page.getByText(tk("ui.check.unavailable"))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.check.restart") }).click();
    await expect(steps(page).locator("[aria-current=step]")).toContainText(tk("ui.check.steps.game"));
  });

  test("після запису видно смужку кроків і верх розповіді (картка не прокручена всередині)", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, gpuResult);
    await page.waitForTimeout(600);
    expect(await page.evaluate(() => Math.max(...[...document.querySelectorAll("section.story *")].map((e) => e.scrollTop)))).toBe(0);
    await expect(stepsBar(page)).toBeInViewport();
    await expect(page.getByRole("region", { name: tk("ui.story.label") }).getByText(tk("diag.story.outcome.fine"))).toBeInViewport();
  });

  test("два фонові процеси з тим самим id — обирається саме натиснутий", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, twoBackground());
    const story = page.getByRole("region", { name: tk("ui.story.label") });
    const discord = story.locator("li").filter({ hasText: "discord.exe" });
    await discord.getByRole("button", { name: tk("ui.story.try") }).click();
    await expect(story.getByText(tk("ui.story.tried"))).toHaveCount(1);
    await expect(discord).toContainText(tk("ui.story.tried"));
    await expect(page.getByRole("region", { name: tk("ui.check.do_title") })).toContainText("discord.exe");
    await page.getByRole("button", { name: tk("ui.check.recheck") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(2);
    expect((await starts(page))[1]).toMatchObject({
      checkOf: "base1",
      checkFinding: { id: "background", advice: { key: "diag.background.advice", args: { name: "discord.exe" } } },
    });
  });

  test("на «стало» блок показує дію «було», а не нову головну", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, fixable());
    await page.getByRole("button", { name: tk("ui.check.recheck") }).click();
    await expect.poll(async () => (await starts(page)).length).toBe(2);
    const after = fixable();
    if (after.diagnosis.story) after.diagnosis.story.action = 2;
    await emit(page, "diag-done", after);
    await emit(page, "history-saved", "after1");
    const block = page.getByRole("region", { name: tk("ui.check.do_title") });
    await expect(block).toContainText(tk("diag.gpu_bound.advice"));
    await expect(block).not.toContainText(tk("diag.upscale.helps.advice"));
  });

  test("«Усе гаразд» з порадами — підказка, що робити далі", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, gpuResult);
    await expect(page.getByText(tk("ui.check.pick_hint"))).toBeVisible();
  });

  test("після «Спробувати» фокус — на «Я зробив», кнопка одна", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    await recordFirst(page, gpuResult);
    const row = page.getByRole("region", { name: tk("ui.story.label") }).locator("li").filter({ hasText: tk("diag.upscale.helps.advice") });
    await row.getByRole("button", { name: tk("ui.story.try") }).click();
    await expect(page.getByRole("button", { name: tk("ui.check.recheck") })).toHaveCount(1);
    await expect(page.getByRole("button", { name: tk("ui.check.recheck") })).toBeFocused();
  });

  test("завершальна картка — без зайвої «Почати запис»", async ({ page }) => {
    await mockBackend(page);
    await startScenario(page);
    const r = structuredClone(gpuResult);
    if (r.diagnosis.story) r.diagnosis.story.others = [];
    await recordFirst(page, r);
    await expect(page.getByRole("region", { name: tk("ui.check.done_title") })).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.diag.start") })).toHaveCount(0);
  });
});
