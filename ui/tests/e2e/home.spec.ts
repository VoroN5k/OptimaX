import { expect, test } from "@playwright/test";
import { calls, mockBackend, unmocked } from "./mock";
import { tk } from "./text";

test.describe("Головна", () => {
  test("чотири плитки «Стан ПК» з даних бекенду", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await expect(page.getByRole("heading", { level: 1, name: tk("ui.home.title") })).toBeVisible();
    const tiles = page.locator("[data-tour=home-state] button");
    await expect(tiles).toHaveCount(4);
    await expect(tiles.nth(0)).toContainText(tk("diag.summary.gpu.title", { game: "re9.exe" }));
    await expect(tiles.nth(0)).toContainText(tk("ui.ago.day", { n: 2 }));
    await expect(tiles.nth(1)).toContainText(tk("ui.home.config_n", { n: 2 }));
    await expect(tiles.nth(2)).toContainText("12.3 ГБ");
    await expect(tiles.nth(3)).toContainText(tk("audit.driver.aging.title", { days: 190 }));
    // Аудит і скан — з кешу (refresh: false); повторюються лише кнопкою.
    const cmds = await calls(page);
    expect(cmds).toContainEqual({ cmd: "dashboard_config", args: { refresh: false } });
    expect(cmds).toContainEqual({ cmd: "dashboard_disk", args: { refresh: false } });
    expect(await unmocked(page)).toEqual([]);
  });

  test("«Оновити» повторює аудит і скан", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.home.refresh") }).click();
    await expect
      .poll(async () => (await calls(page)).filter((c) => c.cmd === "dashboard_disk").map((c) => c.args))
      .toContainEqual({ refresh: true });
  });

  test("без записів — пропозиція записати гру, плитка веде в діагностику", async ({ page }) => {
    await mockBackend(page, { dashboard_last_game: null });
    await page.goto("/");
    const tile = page.locator("[data-tour=home-state] button").first();
    await expect(tile).toContainText(tk("ui.home.game_none"));
    await tile.click();
    await expect(page.getByRole("heading", { level: 1, name: tk("ui.diag.title") })).toBeVisible();
  });

  test("знахідки конфігурації компактні: перша розгорнута, друга — лише заголовок", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await expect(page.getByText(tk("audit.ram.single_module.advice", { slots: 4 }), { exact: false })).toBeVisible();
    const driverDetail = tk("audit.driver.aging.detail", { gpu: "GeForce RTX 4070", version: "566.36", date: "2026-04-26" });
    // Опис драйвера видно лише на плитці (у згорнутій картці його немає).
    await expect(page.locator(".list").getByText(driverDetail)).toHaveCount(0);
  });
});
