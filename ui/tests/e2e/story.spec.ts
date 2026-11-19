import { expect, test, type Page } from "@playwright/test";
import type { DiagnosisResult } from "../../src/lib/bindings/DiagnosisResult";
import { gpuResult } from "./fixtures";
import { mockBackend } from "./mock";
import { go } from "./nav";
import { tk } from "./text";

// Розповідь простими словами зверху результату (docs/superpowers/specs/2026-11-18-plain-verdict-design.md):
// «Як іде гра / Чому були ривки / Що зробити»; інженерний вердикт — у згортці «Детальніше».
async function show(page: Page, last: DiagnosisResult) {
  await mockBackend(page, { diag_state: { recording: false, last } });
  await page.goto("/");
  await go(page, "diagnosis");
}

const card = (page: Page) => page.getByRole("region", { name: tk("ui.story.label") });
const details = (page: Page) => page.getByRole("button", { name: new RegExp(`^${tk("ui.diag.details")}`) });

/** Результат із ривками й причиною: головна дія — порада gpu_bound (A/B), апскейл — серед інших. */
function stuttery(): DiagnosisResult {
  const r = structuredClone(gpuResult);
  r.diagnosis.story = {
    smoothness: "noticeable",
    outcome: "fixable",
    how: [
      { key: "diag.story.how.fps_hz", args: { avg: "125", hz: "200" } },
      { key: "diag.story.how.noticeable", args: { low: "57", avg: "125", spikes: "10" } },
      { key: "diag.story.how.gpu", args: {} },
    ],
    why: [
      {
        code: "shader",
        likely: true,
        hits: 7,
        of: 10,
        text: { key: "diag.story.why.likely.shader", args: { hits: "7", of: "10" } },
      },
    ],
    why_unknown: false,
    action: 1,
    others: [2],
  };
  return r;
}

test.describe("Розповідь простими словами", () => {
  test("видно одразу: як іде гра й підсумок; інженерний вердикт згорнуто", async ({ page }) => {
    await show(page, gpuResult);
    await expect(card(page)).toContainText(tk("diag.story.how.fps_hz", { avg: "125", hz: "200" }));
    await expect(card(page)).toContainText(tk("diag.story.outcome.fine"));
    await expect(card(page)).toContainText(tk("diag.story.smooth.smooth"));
    // «Усе гаразд», але поради є — одразу список «Спробуй», без «Нічого не треба» і без згортки.
    await expect(card(page)).toContainText(tk("ui.story.try_title"));
    await expect(card(page).getByText(tk("diag.upscale.helps.advice"))).toBeVisible();
    await expect(card(page).getByText(tk("ui.story.others"), { exact: false })).toHaveCount(0);
    // Поза сценарієм «Перевірити мою гру» кнопки «Спробувати» немає.
    await expect(card(page).getByRole("button", { name: tk("ui.story.try") })).toHaveCount(0);
    // Ривків немає — блоку «Чому» немає.
    await expect(card(page).getByText(tk("ui.story.why"))).toHaveCount(0);
    await expect(page.getByRole("region", { name: tk("ui.diag.kpi.label") })).toHaveCount(0);
    await expect(details(page)).toHaveAttribute("aria-expanded", "false");
  });

  test("«Детальніше» розгортає смугу показників і головний вердикт", async ({ page }) => {
    await show(page, gpuResult);
    await details(page).click();
    await expect(page.getByRole("region", { name: tk("ui.diag.kpi.label") })).toContainText("125.0");
    await expect(page.getByRole("heading", { name: tk("diag.summary.gpu.title", { game: "re9.exe" }) })).toBeVisible();
  });

  test("ривки з причиною: «найімовірніше», головна дія з кнопкою A/B", async ({ page }) => {
    await show(page, stuttery());
    await expect(card(page)).toContainText(tk("diag.story.smooth.noticeable"));
    await expect(card(page)).toContainText(tk("diag.story.why.likely.shader", { hits: "7", of: "10" }));
    await expect(card(page)).toContainText(tk("diag.story.outcome.fixable"));
    const todo = card(page).locator("li").filter({ hasText: tk("diag.gpu_bound.advice") });
    await expect(todo).toHaveCount(1);
    await todo.getByRole("button", { name: tk("ui.diag.next.compare") }).click();
    await expect(page.getByRole("tab", { name: tk("ui.exp.tab"), selected: true })).toBeVisible();
  });

  test("«Інші варіанти» згорнуті й містять решту порад", async ({ page }) => {
    await show(page, stuttery());
    const upscale = card(page).getByText(tk("diag.upscale.helps.advice"));
    await expect(upscale).toBeHidden();
    await card(page).getByText(tk("ui.story.others"), { exact: false }).click();
    await expect(upscale).toBeVisible();
    await expect(card(page).getByRole("button", { name: tk("ui.up.open") })).toBeVisible();
  });

  test("ривки без причини — так і сказано", async ({ page }) => {
    const r = stuttery();
    if (!r.diagnosis.story) throw new Error("story");
    r.diagnosis.story.why = [];
    r.diagnosis.story.why_unknown = true;
    r.diagnosis.story.outcome = "unexplained";
    await show(page, r);
    await expect(card(page)).toContainText(tk("diag.story.why.unknown"));
    await expect(card(page)).toContainText(tk("diag.story.outcome.unexplained"));
  });

  test("без розповіді (старий результат) — звичний вигляд без згортки", async ({ page }) => {
    const r = structuredClone(gpuResult);
    r.diagnosis.story = null;
    await show(page, r);
    await expect(card(page)).toHaveCount(0);
    await expect(page.getByRole("region", { name: tk("ui.diag.kpi.label") })).toContainText("125.0");
    await expect(details(page)).toHaveCount(0);
  });

  test("у «Детальніше» поради — у картках знахідок, без другого «Що робити далі»", async ({ page }) => {
    await show(page, stuttery());
    await details(page).click();
    await expect(page.getByText(tk("ui.diag.check"), { exact: true })).toHaveCount(0);
    const findings = page.getByRole("region", { name: tk("ui.diag.findings") });
    await expect(findings.getByText(tk("diag.gpu_bound.advice"))).toBeVisible();
    await expect(findings.getByRole("button", { name: tk("ui.diag.next.compare") })).toBeVisible();
  });

  test("запис з «Історії» теж показує розповідь, без підказки «запиши ще раз»", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await go(page, "diagnosis");
    await page.getByRole("tab", { name: tk("ui.history.tab") }).click();
    await page.getByRole("button", { name: tk("ui.history.open") }).first().click();
    await expect(card(page)).toContainText(tk("diag.story.outcome.fine"));
    await expect(page.getByText(tk("ui.diag.next.rerecord"))).toHaveCount(0);
  });
});
