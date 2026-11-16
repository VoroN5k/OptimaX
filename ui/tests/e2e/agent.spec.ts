import { expect, test, type Page } from "@playwright/test";
import { agentReady } from "./fixtures";
import { calls, mockBackend, unmocked } from "./mock";
import { tk } from "./text";

async function open(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: tk("ui.nav.settings") }).click();
  await expect(page.getByText(tk("ui.agent.title"))).toBeVisible();
}

test.describe("Агент у треї", () => {
  test("автозапуск — лише після згоди; запуск зараз", async ({ page }) => {
    await mockBackend(page, { agent_status: agentReady });
    await open(page);
    await expect(page.getByText(tk("ui.agent.state.stopped"), { exact: true })).toBeVisible();
    const toggle = page.getByRole("switch", { name: tk("ui.agent.autostart") });
    await toggle.click();
    await expect(page.getByText(tk("ui.agent.consent.where"))).toBeVisible();
    await page.getByRole("dialog").getByRole("button", { name: tk("ui.common.cancel") }).click();
    await expect(toggle).toHaveAttribute("aria-checked", "false");
    expect((await calls(page)).some((c) => c.cmd === "agent_set_autostart")).toBe(false);

    await toggle.click();
    await page.getByRole("dialog").getByRole("button", { name: tk("ui.agent.consent.ok") }).click();
    await expect(toggle).toHaveAttribute("aria-checked", "true");
    await expect(page.getByText(tk("ui.agent.autostart_on"))).toBeVisible();
    const set = (await calls(page)).find((c) => c.cmd === "agent_set_autostart");
    expect((set?.args as { on?: boolean })?.on).toBe(true);

    await page.getByRole("button", { name: tk("ui.agent.start") }).click();
    await expect(page.getByText(tk("ui.agent.state.running"), { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.agent.start") })).toHaveCount(0);
    expect(await unmocked(page)).toEqual([]);
  });

  test("портативна копія — пояснення замість перемикача", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await expect(page.getByText(tk("ui.agent.unavailable"))).toBeVisible();
    await expect(page.getByRole("switch", { name: tk("ui.agent.autostart") })).toHaveCount(0);
  });

  test("програма з правами адміністратора — агента звідси не запустити", async ({ page }) => {
    await mockBackend(page, { agent_status: { ...agentReady, can_start: false } });
    await open(page);
    await expect(page.getByText(tk("ui.agent.elevated"))).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.agent.start") })).toHaveCount(0);
  });
});
