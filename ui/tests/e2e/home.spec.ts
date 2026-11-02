import { expect, test } from "@playwright/test";
import { calls, mockBackend, unmocked } from "./mock";
import { comparison, driverCheck, historyEntry } from "./fixtures";
import { tk } from "./text";

test.describe("Головна", () => {
  test("чотири плитки «Стан ПК» з даних бекенду", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await expect(page.getByRole("heading", { level: 1, name: tk("ui.home.title") })).toBeVisible();
    const tiles = page.locator("[data-tour=home-state] button.tile");
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
    const tile = page.locator("[data-tour=home-state] button.tile").first();
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
  test("драйвер: перевірка на nvidia.com лише за кнопкою, сторінка — з відповіді", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await expect(page.getByText(tk("ui.driver_online.hint"))).toBeVisible();
    expect((await calls(page)).some((c) => c.cmd === "driver_check")).toBe(false);
    await page.getByRole("button", { name: tk("ui.driver_online.check") }).click();
    await expect(page.getByText(tk("ui.driver_online.newer", { v: "621.01", date: "2026-10-21", current: "566.36" }))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.driver_online.open_page") }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "driver_open_page")?.args)
      .toEqual({ url: driverCheck.page });
  });

  test("драйвер: встановлено останній — без кнопки сторінки", async ({ page }) => {
    await mockBackend(page, { driver_check: { ...driverCheck, installed: "621.01", newer: false } });
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.driver_online.check") }).click();
    await expect(page.getByText(tk("ui.driver_online.latest", { v: "621.01" }))).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.driver_online.open_page") })).toHaveCount(0);
  });

  test("ефект оновлення драйвера з історії веде до порівняння", async ({ page }) => {
    const c = structuredClone(comparison);
    c.trend = "better";
    c.avg_fps_change_pct = 3.2;
    await mockBackend(page, { history_driver_effects: [{ from: "566.36", to: "572.16", comparison: c }] });
    await page.goto("/");
    await expect(page.getByText(tk("ui.driver_online.effect", { from: "566.36", to: "572.16" }))).toBeVisible();
    await page.getByRole("button", { name: "re9.exe +3 %" }).click();
    await expect
      .poll(async () => (await calls(page)).find((x) => x.cmd === "history_compare")?.args)
      .toEqual({ a: c.older.id, b: c.newer.id });
  });
  test("чорна скринька: підсумок останньої сесії з причинами фризів", async ({ page }) => {
    const ended = Date.now() - 3600_000;
    await mockBackend(page, {
      blackbox_state: {
        active: null,
        last: {
          session: {
            id: "1-re9.exe",
            game: "re9.exe",
            started_unix_ms: ended - 7_800_000,
            ended_unix_ms: ended,
            played_s: 7800,
            paused_s: 0,
            threshold_ms: 100,
            freezes: [],
            state: historyEntry(1, 100, "566.36").state,
          },
          causes: [
            { cause: "shader", count: 9 },
            { cause: "disk", count: 3 },
            { cause: "unknown", count: 2 },
          ],
        },
      },
    });
    await page.goto("/");
    const note = page.getByRole("status").filter({ hasText: "re9.exe, " });
    await expect(note).toContainText(tk("ui.blackbox.hours", { h: 2, m: 10 }));
    await expect(note).toContainText(`${tk("diag.cause_name.shader")} · 9`);
    await expect(note).toContainText(`${tk("ui.blackbox.cause_unknown")} · 2`);
    await note.getByRole("button", { name: tk("ui.blackbox.close") }).click();
    await expect(note).toHaveCount(0);
  });

  test("чорна скринька: сесія, що йде", async ({ page }) => {
    await mockBackend(page, {
      blackbox_state: {
        active: { game: "re9.exe", started_unix_ms: Date.now(), played_s: 720, freezes: 3, paused: false },
        last: null,
      },
    });
    await page.goto("/");
    await expect(page.getByText(tk("ui.blackbox.active", { game: "re9.exe", played: tk("ui.blackbox.minutes", { m: 12 }), n: 3 }))).toBeVisible();
  });
});
