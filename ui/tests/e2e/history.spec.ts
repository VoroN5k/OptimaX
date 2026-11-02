import { expect, test, type Page } from "@playwright/test";
import { calls, mockBackend } from "./mock";
import { tk } from "./text";

async function openHistory(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: tk("ui.nav.diagnosis") }).click();
  await page.getByRole("radio", { name: tk("ui.history.tab") }).click();
  await expect(page.getByRole("heading", { level: 1, name: tk("ui.diag.page.history.title") })).toBeVisible();
}

test.describe("Історія записів", () => {
  test("записи гри зі змінами відносно попереднього; інша гра — окремо", async ({ page }) => {
    await mockBackend(page);
    await openHistory(page);
    const rows = page.locator("ul.rows > li");
    await expect(rows).toHaveCount(2);
    await expect(rows.first()).toContainText(tk("ui.history.fps", { avg: 108, low: 76 }));
    await expect(rows.first()).toContainText(tk("ui.history.field.driver"));
    await page.getByRole("radio", { name: "cs2.exe" }).click();
    await expect(rows).toHaveCount(1);
  });

  test("порівняння двох записів: падіння FPS і що змінилося", async ({ page }) => {
    await mockBackend(page);
    await openHistory(page);
    const compare = page.getByRole("button", { name: tk("ui.history.compare") });
    await expect(compare).toBeDisabled();
    const boxes = page.locator("ul.rows input[type=checkbox]");
    await boxes.nth(0).check();
    await boxes.nth(1).check();
    await compare.click();
    await expect.poll(async () => (await calls(page)).some((c) => c.cmd === "history_compare")).toBe(true);
    await expect(page.getByText(tk("ui.history.trend.worse", { t: 8 }))).toBeVisible();
    await expect(page.locator("dl.changes")).toContainText("566.36 → 572.16");
  });

  test("відкриття старого запису показує результат з поміткою", async ({ page }) => {
    await mockBackend(page);
    await openHistory(page);
    await page.getByRole("button", { name: tk("ui.history.open") }).first().click();
    await expect(page.getByText(tk("diag.summary.gpu.title", { game: "re9.exe" })).last()).toBeVisible();
    await expect(page.getByRole("note")).toContainText(tk("ui.history.close"));
  });

  test("видалення — лише після підтвердження", async ({ page }) => {
    await mockBackend(page);
    await openHistory(page);
    await page.getByRole("button", { name: tk("ui.history.delete") }).first().click();
    const dialog = page.getByRole("dialog", { name: tk("ui.history.delete_title") });
    await expect(dialog).toBeVisible();
    expect((await calls(page)).some((c) => c.cmd === "history_delete")).toBe(false);
    await dialog.getByRole("button", { name: tk("ui.history.delete") }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "history_delete")?.args)
      .toEqual({ id: "1793500000000-re9.exe" });
  });

  test("порожня історія — пояснення", async ({ page }) => {
    await mockBackend(page, { history_list: { rows: [], broken: 0 } });
    await openHistory(page);
    await expect(page.getByText(tk("ui.history.empty_title"))).toBeVisible();
  });
});
