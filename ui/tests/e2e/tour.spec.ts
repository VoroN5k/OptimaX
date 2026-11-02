import { expect, test } from "@playwright/test";
import { settings } from "./fixtures";
import { calls, mockBackend } from "./mock";
import { tk } from "./text";

const fresh = { get_settings: { ...settings, onboarded: false } };

type SetSettings = { settings: { onboarded: boolean } };

test.describe("Знайомство", () => {
  test("показується при першому запуску; пропуск зберігає onboarded", async ({ page }) => {
    await mockBackend(page, fresh);
    await page.goto("/");
    const bubble = page.getByRole("dialog", { name: tk("ui.tour.welcome.title") });
    await expect(bubble).toBeVisible();
    await expect(bubble).toContainText(tk("ui.tour.welcome.f1.title"));
    await page.getByRole("button", { name: tk("ui.tour.skip") }).click();
    await expect(bubble).toHaveCount(0);
    await expect
      .poll(async () =>
        (await calls(page)).some((c) => c.cmd === "set_settings" && (c.args as SetSettings).settings.onboarded),
      )
      .toBe(true);
  });

  test("не показується, якщо вже пройдено", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await expect(page.getByRole("heading", { level: 1, name: tk("ui.home.title") })).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });

  test("повний прохід: розділ 3 без запису показує позначений приклад, після — ні", async ({ page }) => {
    await mockBackend(page, fresh);
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.tour.begin") }).click();
    const steps = ["home", "game", "duration", "start_mode", "start", "verdict", "numbers", "findings", "help", "done"];
    for (const id of steps) {
      await expect(page.getByRole("dialog", { name: tk(`ui.tour.${id}.title`) })).toBeVisible();
      if (id === "verdict") await expect(page.getByText(tk("ui.tour.example_title"))).toBeVisible();
      if (id === "done") break;
      await page.getByRole("button", { name: tk("ui.tour.next"), exact: true }).click();
    }
    await page.getByRole("button", { name: tk("ui.tour.finish") }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByText(tk("ui.tour.example_title"))).toHaveCount(0);
  });

  test("Esc пропускає знайомство", async ({ page }) => {
    await mockBackend(page, fresh);
    await page.goto("/");
    await expect(page.getByRole("dialog", { name: tk("ui.tour.welcome.title") })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });
});
