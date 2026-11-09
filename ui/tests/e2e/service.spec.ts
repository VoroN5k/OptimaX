import { expect, test, type Page } from "@playwright/test";
import { serviceNone, serviceRunning } from "./fixtures";
import { calls, mockBackend, unmocked } from "./mock";
import { tk } from "./text";

async function open(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: tk("ui.nav.settings") }).click();
  await expect(page.getByText(tk("ui.service.title"))).toBeVisible();
}

test.describe("Служба OptimaX", () => {
  test("встановлення лише після згоди з описом прав і ризику", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await expect(page.getByText(tk("ui.service.state.not_installed"))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.service.install") }).click();
    await expect(page.getByText(tk("ui.service.consent.risk"))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.common.cancel") }).click();
    expect((await calls(page)).some((c) => c.cmd === "service_install")).toBe(false);
    await page.getByRole("button", { name: tk("ui.service.install") }).click();
    await page.getByRole("dialog").getByRole("button", { name: tk("ui.service.consent.ok") }).click();
    await expect(page.getByText(tk("ui.service.ping", { v: "0.2.0", s: 12 }), { exact: false })).toBeVisible();
    expect(await unmocked(page)).toEqual([]);
  });

  test("копія не з Program Files — пояснення, чому служба її не прийме", async ({ page }) => {
    await mockBackend(page, { service_status: { ...serviceRunning, from_service_exe: false, ping: null } });
    await open(page);
    await expect(page.getByText(tk("ui.service.other_exe", { exe: serviceNone.exe ?? "" }))).toBeVisible();
  });

  test("інша версія служби — пропонує переустановити; видалення з підтвердженням", async ({ page }) => {
    await mockBackend(page, {
      service_status: { ...serviceRunning, ping: { version: "0.1.9", uptime_s: 5 }, same_version: false },
    });
    await open(page);
    await expect(page.getByText(tk("ui.service.version_mismatch", { v: "0.1.9" }))).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.service.reinstall") })).toBeVisible();
    await page.getByRole("button", { name: tk("ui.service.uninstall") }).click();
    await expect(page.getByText(tk("ui.service.remove.text"))).toBeVisible();
    await page.getByRole("dialog").getByRole("button", { name: tk("ui.service.uninstall") }).click();
    await expect.poll(async () => (await calls(page)).some((c) => c.cmd === "service_uninstall")).toBe(true);
    await expect(page.getByText(tk("ui.service.state.not_installed"))).toBeVisible();
  });
});
