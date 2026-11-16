import { expect, test, type Page } from "@playwright/test";
import { cDrive, cleanScan } from "./cleanupFixture";
import { calls, mockBackend } from "./mock";
import { go } from "./nav";
import { tk } from "./text";

// Екран «Очищення» (CLAUDE.md 11.8): підсумок за правилами, вибір лише 🟢/🟡, 🔴 і ⚪ без чекбокса.
/** Тестовий `tk` прибирає зворотні слеші з аргументів — шлях підставляємо після перекладу. */
const withPath = (key: string, path: string) => tk(key, { path: "@@" }).replace("@@", path);

async function open(page: Page) {
  await mockBackend(page, { cleanup_last_scan: cleanScan, cleanup_drives: [cDrive] });
  await page.goto("/");
  await go(page, "cleanup");
}

test.describe("Очищення", () => {
  test("підсумок: знайдено безпечно за правилами, склад диска, категорії в одній плитці", async ({ page }) => {
    await open(page);
    await expect(page.getByText(tk("ui.clean.found_label"))).toBeVisible();
    await expect(page.locator(".sum .big")).toHaveText("9.4 ГБ");
    await expect(page.getByRole("img", { name: /^Диск: безпечно за правилами 9.4 ГБ/ })).toBeVisible();
    // Порожні категорії — одним рядком, не окремими плитками.
    await expect(page.getByText(/За правилами нічого не знайдено: .*Кеші розробника/)).toBeVisible();
    await expect(page.getByRole("heading", { level: 2, name: tk("ui.clean.cat.dev_caches") })).toHaveCount(0);
    // Категорія лише з 🔴 — без «можна очистити 0 Б».
    await expect(page.getByText(tk("ui.clean.cat_info", { n: 1 }))).toBeVisible();
  });

  test("«Вибрати все безпечне» бере лише 🟢, а 🟡 — ні; повторне натискання знімає вибір", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: tk("ui.clean.select_safe", { size: "9.4 ГБ" }) }).click();
    const box = (path: string) => page.getByRole("checkbox", { name: withPath("ui.clean.select", path) });
    await expect(box("C:\\Users\\me\\AppData\\Local\\Temp")).toBeChecked();
    await expect(box("C:\\Users\\me\\AppData\\Local\\CrashDumps")).toBeChecked();
    await expect(box("C:\\Users\\me\\AppData\\Local\\NVIDIA\\DXCache")).not.toBeChecked();
    await expect(page.getByRole("button", { name: tk("ui.clean.clean_selected", { size: "9.4 ГБ" }) })).toBeEnabled();
    await page.getByRole("button", { name: tk("ui.clean.unselect_safe") }).click();
    await expect(box("C:\\Users\\me\\AppData\\Local\\Temp")).not.toBeChecked();
    expect((await calls(page)).some((c) => c.cmd === "cleanup_execute")).toBe(false);
  });

  test("🔴 і ⚪ — без чекбокса, лише «Показати в Провіднику»", async ({ page }) => {
    await open(page);
    for (const path of ["C:\\Windows\\System32\\DriverStore\\FileRepository", "C:\\Games\\old-mods"]) {
      await expect(page.getByRole("checkbox", { name: withPath("ui.clean.select", path) })).toHaveCount(0);
      await expect(page.getByRole("button", { name: withPath("ui.clean.reveal_path", path) })).toBeVisible();
    }
  });

  test("«Деталі» показують правило, джерело й дату перевірки", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: withPath("ui.clean.details_of", "C:\\Users\\me\\AppData\\Local\\Temp") }).click();
    await expect(page.getByText("user_temp", { exact: true })).toBeVisible();
    await expect(page.getByText("Windows 11 24H2, 2026-10-30")).toBeVisible();
  });

  test("перемикання вкладки, прокрутивши нижче, не стрибає на підсумок", async ({ page }) => {
    await page.setViewportSize({ width: 1240, height: 700 });
    await open(page);
    // Як людина: прокрутила так, що підсумок уже за верхнім краєм, а вкладки видно вгорі.
    const before = await page.locator(".page .scroll").evaluate((el) => {
      const tabs = el.querySelector(".tabs-row") as HTMLElement;
      const y = tabs.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop;
      el.scrollTop = y - 60;
      return el.scrollTop;
    });
    // Покадрово: найменша позиція прокрутки під час перемикання.
    await page.evaluate(() => {
      const w = window as unknown as { __minTop: number };
      const el = document.querySelector(".page .scroll") as HTMLElement;
      w.__minTop = el.scrollTop;
      const t0 = performance.now();
      const probe = () => {
        w.__minTop = Math.min(w.__minTop, el.scrollTop);
        if (performance.now() - t0 < 1200) requestAnimationFrame(probe);
      };
      requestAnimationFrame(probe);
    });
    await page.getByRole("radio", { name: tk("ui.clean.tab.growth") }).click();
    await page.waitForTimeout(1300);
    const minTop = await page.evaluate(() => (window as unknown as { __minTop: number }).__minTop);
    expect(minTop).toBeGreaterThanOrEqual(before - 2);
  });
});
