import { expect, test, type Page } from "@playwright/test";
import { settings } from "./fixtures";
import { calls, emit, mockBackend } from "./mock";
import { go } from "./nav";
import { tk } from "./text";

// «Ефекти» (docs/DESIGN.md розд. 8, CLAUDE.md 10.5): рівні, палітра, контракт вимірювань,
// згорнуте вікно. Стан видно в атрибутах <html>, які читають компоненти в CSS.
const html = (page: Page) => page.locator("html");

/** Обчислене значення CSS-властивості першого елемента з нескінченною анімацією в DOM. */
async function infiniteProbe(page: Page, prop: "animationIterationCount") {
  return page.evaluate((prop) => {
    const el = document.createElement("div");
    el.style.animation = "x 1s linear infinite";
    document.body.append(el);
    const v = getComputedStyle(el)[prop];
    el.remove();
    return v;
  }, prop);
}

test.describe("Ефекти", () => {
  test("«Як у системі»: зменшений рух → Вимкнені; звичайний → Повні", async ({ page }) => {
    await mockBackend(page, { get_settings: { ...settings, effects: null } });
    await page.goto("/");
    await expect(html(page)).toHaveAttribute("data-effects", "off");
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await expect(html(page)).not.toHaveAttribute("data-effects", /.+/);
  });

  test("налаштування: рівень і палітра зберігаються й діють одразу", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await mockBackend(page, { get_settings: { ...settings, effects: null } });
    await page.goto("/");
    await go(page, "settings");
    const level = page.getByRole("radiogroup", { name: tk("ui.settings.effects") });
    await expect(page.getByText(tk("ui.settings.effects.system_full"))).toBeVisible();
    await level.getByRole("radio", { name: tk("ui.settings.effects.calm") }).click();
    await expect(html(page)).toHaveAttribute("data-effects", "calm");
    expect(await infiniteProbe(page, "animationIterationCount")).toBe("1");
    await page.getByRole("radiogroup", { name: tk("ui.settings.palette") })
      .getByRole("radio", { name: tk("ui.settings.palette.aurora") })
      .click();
    await expect(html(page)).toHaveAttribute("data-palette", "aurora");
    const saved = (await calls(page)).filter((c) => c.cmd === "set_settings").at(-1);
    expect(saved?.args).toMatchObject({ settings: { effects: "calm", palette: "aurora" } });
    await level.getByRole("radio", { name: tk("ui.settings.effects.full") }).click();
    await expect(html(page)).not.toHaveAttribute("data-effects", /.+/);
    expect(await infiniteProbe(page, "animationIterationCount")).toBe("infinite");
  });

  test("під час запису нескінченні анімації зупиняються навіть у «Повних»", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await mockBackend(page, { get_settings: { ...settings, effects: "full" }, diag_state: { recording: true, last: null } });
    await page.goto("/");
    await go(page, "diagnosis");
    await expect(html(page)).toHaveClass(/\bmeasuring\b/);
    expect(await infiniteProbe(page, "animationIterationCount")).toBe("1");
    await emit(page, "diag-error", { kind: "other", message: "x" });
    await expect(html(page)).not.toHaveClass(/\bmeasuring\b/);
    expect(await infiniteProbe(page, "animationIterationCount")).toBe("infinite");
  });

  test("згорнуте вікно: нескінченні анімації зупиняються", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await mockBackend(page, { get_settings: { ...settings, effects: "full" } });
    await page.goto("/");
    const setHidden = (hidden: boolean) =>
      page.evaluate((hidden) => {
        Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
        document.dispatchEvent(new Event("visibilitychange"));
      }, hidden);
    await setHidden(true);
    await expect(html(page)).toHaveClass(/\bwindow-hidden\b/);
    expect(await infiniteProbe(page, "animationIterationCount")).toBe("1");
    await setHidden(false);
    await expect(html(page)).not.toHaveClass(/\bwindow-hidden\b/);
    expect(await infiniteProbe(page, "animationIterationCount")).toBe("infinite");
  });

  test("вікно без фокуса (наприклад, поверх гра): нескінченні анімації зупиняються, а вміст нового розділу з’являється", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await mockBackend(page, { get_settings: { ...settings, effects: "full" } });
    await page.goto("/");
    // Паралельні сторінки тестів можуть стартувати без фокуса — спершу явно «активне вікно».
    await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await expect(html(page)).not.toHaveClass(/\bwindow-hidden\b/);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect(html(page)).toHaveClass(/\bwindow-hidden\b/);
    expect(await infiniteProbe(page, "animationIterationCount")).toBe("1");
    // Регресія: одноразова поява сторінки не замерзає на прозорому першому кадрі.
    await go(page, "settings");
    const pageEl = page.locator(".page").first();
    await expect.poll(() => pageEl.evaluate((el) => Number(getComputedStyle(el).opacity))).toBe(1);
    await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await expect(html(page)).not.toHaveClass(/\bwindow-hidden\b/);
  });

  test("простій 20 с без введення: нескінченні анімації зупиняються до першого руху миші", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await page.clock.install();
    await mockBackend(page, { get_settings: { ...settings, effects: "full" } });
    await page.goto("/");
    await expect(html(page)).not.toHaveClass(/\bidle\b/);
    await page.clock.fastForward(21_000);
    await expect(html(page)).toHaveClass(/\bidle\b/);
    expect(await infiniteProbe(page, "animationIterationCount")).toBe("1");
    await page.mouse.move(200, 200);
    await page.mouse.move(260, 240);
    await expect(html(page)).not.toHaveClass(/\bidle\b/);
  });
});
