import { expect, test, type Page } from "@playwright/test";
import { gpuResult, library } from "./fixtures";
import { calls, emit, mockBackend } from "./mock";
import { go } from "./nav";
import { tk } from "./text";

// Бібліотека ігор у «Діагностиці»: гра зі списку замість вибору процесу.
async function open(page: Page) {
  await page.goto("/");
  await go(page, "diagnosis");
  await expect(page.getByRole("heading", { level: 1, name: tk("ui.diag.title") })).toBeVisible();
}

const grid = (page: Page) => page.getByRole("list", { name: tk("ui.lib.tab_games") });
const card = (page: Page, name: string) => grid(page).getByRole("button", { name: new RegExp(name) });

test.describe("Бібліотека ігор", () => {
  test("список ігор з назвами, лаунчером і станом процесу; прихована гра не видна", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    await expect(card(page, "The Witcher 3")).toBeVisible();
    await expect(card(page, "The Witcher 3")).toContainText(tk("ui.lib.launcher.steam"));
    await expect(card(page, "The Witcher 3")).toContainText(tk("ui.lib.exe", { exe: "witcher3.exe" }));
    await expect(card(page, "Red Dead Redemption 2")).toContainText(tk("ui.lib.launcher.epic"));
    await expect(card(page, "Red Dead Redemption 2")).toContainText(tk("ui.lib.exe_unknown"));
    await expect(card(page, "Wallpaper Engine")).toHaveCount(0);
  });

  test("пошук фільтрує за назвою", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    await page.getByRole("searchbox", { name: tk("ui.lib.search") }).fill("red");
    await expect(card(page, "Red Dead Redemption 2")).toBeVisible();
    await expect(card(page, "The Witcher 3")).toHaveCount(0);
    await page.getByRole("searchbox", { name: tk("ui.lib.search") }).fill("zzz");
    await expect(page.getByText(tk("ui.lib.nothing_found", { q: "zzz" }))).toBeVisible();
  });

  test("«Приховати» ховає гру, «Показати приховані» повертає", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    const item = grid(page).getByRole("listitem").filter({ hasText: "The Witcher 3" });
    await item.getByRole("button", { name: tk("ui.lib.hide") }).click();
    await expect(card(page, "The Witcher 3")).toHaveCount(0);
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "library_set_hidden")?.args)
      .toEqual({ id: "steam:292030", hidden: true });
    await page.getByRole("button", { name: tk("ui.lib.show_hidden", { n: 2 }) }).click();
    await expect(card(page, "The Witcher 3")).toBeVisible();
    await expect(card(page, "Wallpaper Engine")).toBeVisible();
  });

  test("порожня бібліотека — одразу вкладка процесів", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await expect(page.locator("[data-tour=diag-game] input").first()).toBeVisible();
    await expect(grid(page)).toHaveCount(0);
  });

  test("гра з відомим процесом — запис одразу за цим exe", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    await card(page, "The Witcher 3").click();
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "diag_start")?.args)
      .toEqual({ exe: "witcher3.exe", seconds: 60, scene: null });
  });

  test("гра без процесу — очікування запуску, потім запис за визначеним exe", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    await card(page, "Red Dead Redemption 2").click();
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toBeVisible();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "library_detect_start")?.args)
      .toEqual({ id: "epic:Heather" });
    // Бекенд визначає гру у власному потоці й повідомляє подією.
    await emit(page, "library-detect", { state: "found", exe: "RDR2.exe" });
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "diag_start")?.args)
      .toEqual({ exe: "RDR2.exe", seconds: 60, scene: null });
  });

  test("вибір іншої гри під час очікування скасовує його", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    await card(page, "Red Dead Redemption 2").click();
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toBeVisible();
    await card(page, "The Witcher 3").click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toHaveCount(0);
    await expect
      .poll(async () => (await calls(page)).some((c) => c.cmd === "library_detect_cancel"))
      .toBe(true);
    await expect(page.getByRole("button", { name: tk("ui.diag.start") })).toBeEnabled();
  });

  test("за хвилину очікування — підказка вибрати процес вручну", async ({ page }) => {
    await page.clock.install();
    await mockBackend(page, { library_scan: library });
    await open(page);
    await card(page, "Red Dead Redemption 2").click();
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toBeVisible();
    await expect(page.getByText(tk("ui.lib.detect_slow"))).toHaveCount(0);
    await page.clock.fastForward(61_000);
    await expect(page.getByText(tk("ui.lib.detect_slow"))).toBeVisible();
  });

  test("пропущений exe видно, «Це все ж гра» повертає його", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    await card(page, "Red Dead Redemption 2").click();
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toBeVisible();
    await emit(page, "library-detect", { state: "waiting", skipped: ["PlayRDR2.exe"] });
    await expect(page.getByText(tk("ui.lib.skipped", { exe: "PlayRDR2.exe" }))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.lib.skipped_undo") }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "library_forget_exe")?.args)
      .toEqual({ id: "epic:Heather" });
  });

  test("визначений процес закрився посеред запису — наступний старт знову чекає на гру", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    await card(page, "Red Dead Redemption 2").click();
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toBeVisible();
    await emit(page, "library-detect", { state: "found", exe: "PlayRDR2.exe" });
    await expect
      .poll(async () => (await calls(page)).filter((c) => c.cmd === "diag_start").length)
      .toBe(1);
    const exited = structuredClone(gpuResult);
    exited.end_reason = "game_exited";
    await emit(page, "diag-done", exited);
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toBeVisible();
    await expect
      .poll(async () => (await calls(page)).filter((c) => c.cmd === "library_detect_start").length)
      .toBe(2);
  });

  test("очікування можна скасувати", async ({ page }) => {
    await mockBackend(page, { library_scan: library });
    await open(page);
    await card(page, "Red Dead Redemption 2").click();
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.common.cancel") }).click();
    await expect(page.getByText(tk("ui.lib.detect_title"))).toHaveCount(0);
    await expect
      .poll(async () => (await calls(page)).some((c) => c.cmd === "library_detect_cancel"))
      .toBe(true);
    expect((await calls(page)).some((c) => c.cmd === "diag_start")).toBe(false);
  });
});
