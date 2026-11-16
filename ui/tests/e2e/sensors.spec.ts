import { expect, test, type Page } from "@playwright/test";
import { calls, mockBackend, unmocked } from "./mock";
import { tk } from "./text";
import { go } from "./nav";

async function open(page: Page) {
  await page.goto("/");
  await go(page, "settings");
  await expect(page.getByText(tk("ui.sensors.title"))).toBeVisible();
}

const reading = {
  cpu_c: 47.5,
  ccd_c: [35],
  clocks: { fclk_mhz: 1800, uclk_mhz: 900, mclk_mhz: 1800 },
  pm_version: 0x380905,
};

test.describe("Розширені датчики (PawnIO)", () => {
  test("без драйвера — лише пояснення й посилання, нічого не вмикається", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await expect(page.getByText(tk("ui.sensors.not_installed"))).toBeVisible();
    await expect(page.getByRole("switch", { name: tk("ui.sensors.enable") })).toHaveCount(0);
    await page.getByRole("button", { name: tk("ui.sensors.open_site") }).click();
    await expect.poll(async () => (await calls(page)).some((c) => c.cmd === "sensors_open_site")).toBe(true);
    expect(await unmocked(page)).toEqual([]);
  });

  test("увімкнення лише після згоди з описом ризиків", async ({ page }) => {
    await mockBackend(page, { sensors_status: { state: "disabled", version: "2.2.0.0" } });
    await open(page);
    await expect(page.getByText(tk("ui.sensors.installed", { v: "2.2.0.0" }))).toBeVisible();
    await page.getByRole("switch", { name: tk("ui.sensors.enable") }).click();
    await expect(page.getByText(tk("ui.sensors.consent.risk"))).toBeVisible();
    // Скасування — налаштування не збережено.
    await page.getByRole("button", { name: tk("ui.common.cancel") }).click();
    await expect(page.getByRole("switch", { name: tk("ui.sensors.enable") })).toHaveAttribute("aria-checked", "false");
    expect((await calls(page)).some((c) => c.cmd === "set_settings")).toBe(false);
    // Згода — зберігається з pawnio_sensors: true.
    await page.getByRole("switch", { name: tk("ui.sensors.enable") }).click();
    await page.getByRole("button", { name: tk("ui.sensors.consent.ok") }).click();
    await expect
      .poll(async () => {
        const s = (await calls(page)).filter((c) => c.cmd === "set_settings").at(-1);
        return (s?.args as { settings?: { pawnio_sensors?: boolean } } | undefined)?.settings?.pawnio_sensors;
      })
      .toBe(true);
  });

  test("показання: температура й частоти шини пам'яті", async ({ page }) => {
    await mockBackend(page, { sensors_status: { state: "ok", version: "2.2.0.0", reading, via_service: true } });
    await open(page);
    await expect(page.getByText("47.5 °C")).toBeVisible();
    await expect(page.getByText(tk("ui.sensors.via_service"))).toBeVisible();
    await expect(page.getByText(tk("ui.sensors.ccd_temp", { n: 1 }))).toBeVisible();
    await expect(page.getByText("1800 / 900 / 1800")).toBeVisible();
  });

  test("невідома таблиця SMU — частот немає, і сказано чому", async ({ page }) => {
    await mockBackend(page, {
      sensors_status: {
        state: "ok",
        version: "2.2.0.0",
        reading: { ...reading, clocks: null, pm_version: 0x540004 },
        via_service: false,
      },
    });
    await open(page);
    await expect(page.getByText(tk("ui.sensors.clocks_unknown", { v: "540004" }))).toBeVisible();
    await expect(page.getByText(tk("ui.sensors.clocks"))).toHaveCount(0);
  });
});
