import { expect, test, type Page } from "@playwright/test";
import { stabDone, stabInfo, stabRunning } from "./fixtures";
import { calls, emit, mockBackend } from "./mock";
import { tk } from "./text";

async function open(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: tk("ui.nav.stability") }).click();
  await expect(page.getByRole("heading", { level: 1, name: tk("ui.stab.title") })).toBeVisible();
}

test.describe("Стабільність", () => {
  test("тест запускається лише після підтвердження з описом наслідків", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await expect(page.getByText(tk("ui.stab.limit", { c: 85 }))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.stab.start") }).click();
    await expect(page.getByText(tk("ui.stab.confirm.gpu.text", { min: 10 }))).toBeVisible();
    expect((await calls(page)).some((c) => c.cmd === "stab_start")).toBe(false);
    await page.getByRole("button", { name: tk("ui.stab.confirm_ok"), exact: true }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "stab_start")?.args)
      .toEqual({ kind: "gpu", seconds: 600, acceptNoSensor: false });
    await expect(page.getByText(tk("ui.stab.running_title"))).toBeVisible();
    await expect(page.getByText(`1:15 / 10:00`)).toBeVisible();
    await page.getByRole("button", { name: tk("ui.stab.stop") }).click();
    await expect.poll(async () => (await calls(page)).some((c) => c.cmd === "stab_stop")).toBe(true);
  });

  test("розбіжність видно одразу, а підсумок каже про помилку", async ({ page }) => {
    await mockBackend(page, { stab_state: { ...stabRunning, mismatches: 2 } });
    await open(page);
    await expect(page.getByText(tk("ui.stab.found_errors", { n: 2 }))).toBeVisible();
    await emit(page, "stab-done", { ...stabDone, outcome: "errors", mismatches: 2 });
    await expect(page.getByText(tk("ui.stab.outcome.errors.title"))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.outcome.errors.text", { n: 2 }))).toBeVisible();
    await expect(page.locator(".result.bad-box")).toHaveCount(1);
  });

  test("без помилок — чесно, що це не доказ стабільності, і події журналу видно", async ({ page }) => {
    await mockBackend(page, { stab_state: stabRunning });
    await open(page);
    await emit(page, "stab-done", stabDone);
    await expect(page.getByText(tk("ui.stab.outcome.no_errors.text", { min: "10.0" }))).toBeVisible();
    await emit(page, "stab-done", {
      ...stabDone,
      outcome: "hardware_events",
      events: { ...stabDone.events, whea_corrected: 3 },
    });
    await expect(page.getByText(tk("ui.stab.outcome.hardware_events.title"))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.ev.whea_corrected", { n: 3 }))).toBeVisible();
  });

  test("без датчика температури запуск вимагає явної згоди", async ({ page }) => {
    await mockBackend(page, { stab_info: { ...stabInfo, gpu: null, has_sensor: false } });
    await open(page);
    await expect(page.getByText(tk("ui.stab.no_sensor"))).toBeVisible();
    const start = page.getByRole("button", { name: tk("ui.stab.start") });
    await expect(start).toBeDisabled();
    await page.getByRole("switch", { name: tk("ui.stab.accept_no_sensor") }).click();
    await start.click();
    await page.getByRole("button", { name: tk("ui.stab.confirm_ok"), exact: true }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "stab_start")?.args)
      .toEqual({ kind: "gpu", seconds: 600, acceptNoSensor: true });
  });

  test("процесор: завжди потрібна згода, бо температури немає", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await page.getByRole("radio", { name: tk("ui.stab.kind_cpu") }).click();
    await expect(page.getByText(tk("ui.stab.no_sensor_cpu"))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.cpu", { name: stabInfo.cpu, cores: 8 }))).toBeVisible();
    const start = page.getByRole("button", { name: tk("ui.stab.start") });
    await expect(start).toBeDisabled();
    await page.getByRole("switch", { name: tk("ui.stab.accept_no_sensor") }).click();
    await start.click();
    await expect(page.getByText(tk("ui.stab.confirm.cpu.text", { min: 10 }))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.stab.confirm_ok"), exact: true }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "stab_start")?.args)
      .toEqual({ kind: "cpu", seconds: 600, acceptNoSensor: true });
  });

  test("процесор: видно поточне ядро, а підсумок називає ядро з помилкою", async ({ page }) => {
    await mockBackend(page, {
      stab_state: { ...stabRunning, kind: "cpu", adapter: stabInfo.cpu, current_core: 3, total_cores: 8, mismatches: 1, bad_cores: [3] },
    });
    await open(page);
    await expect(page.getByText(tk("ui.stab.core_now", { n: 3, total: 8 }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.bad_cores", { cores: "3" }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.temp"))).toHaveCount(0);
    await emit(page, "stab-done", {
      ...stabDone,
      kind: "cpu",
      outcome: "errors",
      mismatches: 1,
      bad_cores: [3],
      total_cores: 8,
      stats: { max_temp_c: null, avg_clock_mhz: null, avg_power_w: null, avg_util_pct: null },
    });
    await expect(page.getByText(tk("ui.stab.outcome.errors.text_cpu", { n: 1 }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.bad_cores", { cores: "3" }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.max_temp"))).toHaveCount(0);
  });

  test("процесор: якщо процес упав, видно ядро, що тоді працювало", async ({ page }) => {
    await mockBackend(page, { stab_state: { ...stabRunning, kind: "cpu", current_core: 5, total_cores: 8 } });
    await open(page);
    await emit(page, "stab-done", {
      ...stabDone,
      kind: "cpu",
      outcome: "worker_crashed",
      last_core: 5,
      total_cores: 8,
    });
    await expect(page.getByText(tk("ui.stab.outcome.worker_crashed.text_cpu"))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.crash_core", { n: 5 }))).toBeVisible();
  });

  test("збережені результати видно й можна видалити", async ({ page }) => {
    await mockBackend(page, { stab_list: [{ ...stabDone, outcome: "interrupted" }] });
    await open(page);
    await expect(page.getByText(tk("ui.stab.saved_title"))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.outcome.interrupted.title"))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.stab.delete") }).click();
    await expect.poll(async () => (await calls(page)).some((c) => c.cmd === "stab_delete")).toBe(true);
  });
});
