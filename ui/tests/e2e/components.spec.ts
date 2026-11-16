import { expect, test } from "@playwright/test";
import { mockBackend } from "./mock";

// Базові компоненти «Pro Premium» на сторінці-галереї для розробки (`?gallery`).
test.describe("Компоненти (галерея)", () => {
  test.beforeEach(async ({ page }) => {
    await mockBackend(page);
    await page.goto("/?gallery");
    await expect(page.getByTestId("gallery")).toBeVisible();
  });

  test("кнопки: клік, неактивна й завантаження не натискаються, фокус видно", async ({ page }) => {
    await page.getByRole("button", { name: "Головна", exact: true }).click();
    await expect(page.getByTestId("clicks")).toHaveText("кліків: 1");
    await expect(page.getByRole("button", { name: "Неактивна", exact: true })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Завантаження" })).toHaveAttribute("aria-busy", "true");
    // Фокус із клавіатури — видиме кільце.
    await page.getByRole("button", { name: "Головна", exact: true }).focus();
    await page.keyboard.press("Tab");
    await page.keyboard.press("Shift+Tab");
    const outline = await page
      .getByRole("button", { name: "Головна", exact: true })
      .evaluate((el) => getComputedStyle(el).outlineStyle);
    expect(outline).toBe("solid");
  });

  test("підвкладки й група кнопок: стани для читачів екрана", async ({ page }) => {
    await page.getByRole("tab", { name: "Історія" }).click();
    await expect(page.getByRole("tab", { name: "Історія" })).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowRight");
    await expect(page.getByRole("tab", { name: "Чому вилітає?" })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("button", { name: "Найгірший ривок" })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByRole("button", { name: "← Попередній" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Наблизити" })).toBeVisible();
  });

  test("порожні дані — «—», а не вигадане число", async ({ page }) => {
    const empty = page.getByRole("region", { name: "Порожній стан" });
    await expect(empty.getByText("—")).toHaveCount(2);
    await expect(page.getByRole("img", { name: "Даних немає" })).toContainText("—");
  });

  test("кільце: повний текстовий зміст; згортка й бічна плитка працюють", async ({ page }) => {
    await expect(page.getByRole("img", { name: /головний потік 93 відсотки, GPU 76/ })).toBeVisible();
    const disc = page.locator("summary", { hasText: "Докази" });
    await disc.click();
    await expect(page.getByText("Вміст згортки.")).toBeVisible();
    await page.getByRole("button", { name: /Бічна знахідка \(кнопка\)/ }).click();
    await expect(page.getByTestId("clicks")).toHaveText("кліків: 1");
  });

  test("пульс — лише для «Помірний вплив» і гірших", async ({ page }) => {
    // warn і bad у галереї мають pulse; ok і neutral — ні (навіть якби його попросили).
    const pulsing = await page.locator(".dot.pulse").count();
    expect(pulsing).toBeGreaterThanOrEqual(2);
    const okPulse = await page.locator(".pill.ok .dot.pulse, .pill.neutral .dot.pulse").count();
    expect(okPulse).toBe(0);
  });

  test("режими ефектів: «Спокійні» зупиняють нескінченний рух і розмиття", async ({ page }) => {
    // Тести йдуть із reduced-motion (там рух вимкнений завжди) — тут потрібен звичайний режим.
    await page.emulateMedia({ reducedMotion: "no-preference" });
    const spin = page.locator(".frame .spin").first();
    // Svelte додає до імені анімації префікс компонента — перевіряємо лише, що вона є.
    expect(await spin.evaluate((el) => getComputedStyle(el).animationName)).toMatch(/spin/);
    await page.getByRole("radio", { name: "Спокійні" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-effects", "calm");
    expect(await spin.evaluate((el) => getComputedStyle(el).animationName)).toBe("none");
    const blur = await page.locator(".tile").first().evaluate((el) => getComputedStyle(el).backdropFilter);
    expect(blur).toBe("none");
    await page.getByRole("radio", { name: "Повні" }).click();
    await expect(page.locator("html")).not.toHaveAttribute("data-effects", /.+/);
  });

  test("палітри змінюють лише акцент", async ({ page }) => {
    const accent = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--a1").trim());
    const warn = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--warn").trim());
    expect(await accent()).toBe("#F3E5C3");
    await page.getByRole("radio", { name: "Аврора" }).click();
    expect(await accent()).toBe("#9A8CFF");
    expect(await warn()).toBe("#FFB27A");
    await page.getByRole("radio", { name: "Смарагд" }).click();
    expect(await accent()).toBe("#7BF0C4");
  });
});
