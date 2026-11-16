import { expect, test } from "@playwright/test";
import { mockBackend } from "./mock";
import { go } from "./nav";
import { tk } from "./text";

test.describe("Навігація: верхня панель", () => {
  test("вкладки, підвкладки й повернення на останній розділ вкладки", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    const nav = page.getByRole("navigation", { name: tk("ui.nav.label") });
    await expect(nav.getByRole("button", { name: tk("ui.tab.overview") })).toHaveAttribute("aria-current", "page");
    await expect(page.getByRole("heading", { level: 1, name: tk("ui.home.title") })).toBeVisible();

    // «Інструменти» → «Очищення»; на «Огляд» і назад — знову «Очищення».
    await go(page, "cleanup");
    await expect(page.getByRole("tab", { name: tk("ui.nav.cleanup"), exact: true })).toHaveAttribute("aria-selected", "true");
    await nav.getByRole("button", { name: tk("ui.tab.overview") }).click();
    await nav.getByRole("button", { name: tk("ui.tab.tools") }).click();
    await expect(page.getByRole("tab", { name: tk("ui.nav.cleanup"), exact: true })).toHaveAttribute("aria-selected", "true");

    // Стрілки між підвкладками.
    await page.getByRole("tab", { name: tk("ui.nav.cleanup"), exact: true }).focus();
    await page.keyboard.press("ArrowRight");
    await expect(page.getByRole("tab", { name: tk("ui.nav.startup"), exact: true })).toHaveAttribute("aria-selected", "true");

    // У «Тестах» один розділ — підвкладок немає.
    await go(page, "stability");
    await expect(page.getByRole("tablist")).toHaveCount(0);
  });

  test("«Програми» й «Вигляд» (ще не готові) у навігації не показуються", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    for (const tab of ["overview", "tools", "system"]) {
      await page.getByRole("navigation", { name: tk("ui.nav.label") }).getByRole("button", { name: tk(`ui.tab.${tab}`) }).click();
      await expect(page.getByRole("tab", { name: tk("ui.nav.apps") })).toHaveCount(0);
      await expect(page.getByRole("tab", { name: tk("ui.nav.appearance") })).toHaveCount(0);
    }
  });

  test("журнал дій — висувна панель із кнопки; Esc закриває", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    const button = page.getByRole("button", { name: tk("ui.log.button_label", { n: 0 }) });
    await button.click();
    await expect(button).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByRole("complementary", { name: tk("ui.log.title") })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("complementary", { name: tk("ui.log.title") })).toHaveCount(0);
  });

  test("пілюля прав: адміністратор / через службу", async ({ page }) => {
    await mockBackend(page, {
      app_info: { version: "0.0.0-test", elevated: false, service: true, presentmon_version: "2.6.0" },
    });
    await page.goto("/");
    await expect(page.getByText(tk("ui.status.service"))).toBeVisible();
  });
});

test.describe("Каркас сторінки", () => {
  test.use({ viewport: { width: 960, height: 640 } });

  test("довга сторінка прокручується", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    await go(page, "settings");
    const scroll = page.locator(".page .scroll");
    const m = await scroll.evaluate((el) => ({ sh: el.scrollHeight, ch: el.clientHeight }));
    expect(m.sh).toBeGreaterThan(m.ch);
    await scroll.evaluate((el) => el.scrollTo(0, 400));
    expect(await scroll.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
  });

  test("підрозділи завжди праворуч у рядку заголовка", async ({ page }) => {
    await mockBackend(page);
    await page.goto("/");
    for (const screen of ["analysis", "diagnosis", "cleanup", "settings"]) {
      await go(page, screen);
      const header = page.locator(".page > .scroll > header");
      const h = await header.boundingBox();
      const t = await header.getByRole("tablist").boundingBox();
      const title = await header.getByRole("heading", { level: 1 }).boundingBox();
      if (!h || !t || !title) throw new Error(`немає шапки на ${screen}`);
      expect(Math.abs(h.x + h.width - (t.x + t.width)), screen).toBeLessThan(2);
      expect(t.y, screen).toBeLessThan(title.y + title.height + 60);
      expect(t.x, screen).toBeGreaterThan(title.x + 200);
    }
  });
});
