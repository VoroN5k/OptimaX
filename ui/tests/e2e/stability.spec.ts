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
    await expect(page.getByText(tk("ui.stab.confirm.gpu.text", { time: tk("ui.stab.minutes", { n: 10 }) }))).toBeVisible();
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
    await expect(page.getByText(tk("ui.stab.confirm.cpu.text", { time: tk("ui.stab.minutes", { n: 10 }) }))).toBeVisible();
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

  test("пам'ять: згода, чесний обсяг і запуск", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await page.getByRole("radio", { name: tk("ui.stab.kind_ram") }).click();
    await expect(page.getByText(tk("ui.stab.ram", { total: 32768, mb: 12000 }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.no_sensor_ram"))).toBeVisible();
    const start = page.getByRole("button", { name: tk("ui.stab.start") });
    await expect(start).toBeDisabled();
    await page.getByRole("switch", { name: tk("ui.stab.accept_no_sensor") }).click();
    await start.click();
    await expect(page.getByText(tk("ui.stab.confirm.ram.text", { time: tk("ui.stab.minutes", { n: 10 }) }))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.stab.confirm_ok"), exact: true }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "stab_start")?.args)
      .toEqual({ kind: "ram", seconds: 600, acceptNoSensor: true });
  });

  test("пам'ять: замало вільної — запуск недоступний і видно чому", async ({ page }) => {
    await mockBackend(page, { stab_info: { ...stabInfo, ram_test_mb: 0 } });
    await open(page);
    await page.getByRole("radio", { name: tk("ui.stab.kind_ram") }).click();
    await expect(page.getByText(tk("ui.stab.ram_low"))).toBeVisible();
    await page.getByRole("switch", { name: tk("ui.stab.accept_no_sensor") }).click();
    await expect(page.getByRole("button", { name: tk("ui.stab.start") })).toBeDisabled();
  });

  test("пам'ять: підсумок каже про перевірений обсяг і не радить про відеокарту", async ({ page }) => {
    const ram = { kind: "ram" as const, adapter: "RAM", tested_mb: 12000, vram_mb: 32768 };
    await mockBackend(page, {
      stab_state: { ...stabRunning, ...ram, mismatches: 4 },
    });
    await open(page);
    await expect(page.getByText(tk("ui.stab.ram_name"))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.found_errors_ram", { n: 4 }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.tested_ram", { mb: 12000, total: 32768 }))).toBeVisible();
    await emit(page, "stab-done", { ...stabDone, ...ram, outcome: "errors", mismatches: 4 });
    await expect(page.getByText(tk("ui.stab.outcome.errors.text_ram", { n: 4 }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.tested_ram", { mb: 12000, total: 32768 }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.max_temp"))).toHaveCount(0);
    await emit(page, "stab-done", { ...stabDone, ...ram });
    await expect(page.getByText(tk("ui.stab.outcome.no_errors.text_ram", { min: "10.0" }))).toBeVisible();
  });

  test("ніч: години, окрема згода для процесора й зупинка на першій помилці", async ({ page }) => {
    await mockBackend(page);
    await open(page);
    await page.getByRole("radio", { name: tk("ui.stab.kind_cpu") }).click();
    await page.getByRole("radio", { name: tk("ui.stab.mode_night") }).click();
    await expect(page.getByRole("radio", { name: tk("ui.stab.hours", { n: 8 }) })).toBeChecked();
    await expect(page.getByText(tk("ui.stab.night_hint"))).toBeVisible();
    const start = page.getByRole("button", { name: tk("ui.stab.start") });
    await page.getByRole("switch", { name: tk("ui.stab.accept_no_sensor") }).click();
    // Згода «без датчика» ще не згода на години без нагляду.
    await expect(start).toBeDisabled();
    await page.getByRole("switch", { name: tk("ui.stab.accept_night") }).click();
    await start.click();
    await expect(page.getByText(tk("ui.stab.confirm.cpu.text", { time: tk("ui.stab.hours", { n: 8 }) }))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.confirm.night"))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.stab.confirm_ok"), exact: true }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "stab_start")?.args)
      .toEqual({ kind: "cpu", seconds: 28800, acceptNoSensor: true });
  });

  test("ніч: хід у годинах, а підсумок каже, що зупинено на першій помилці", async ({ page }) => {
    const night = { planned_s: 28800, elapsed_s: 3725 };
    await mockBackend(page, { stab_state: { ...stabRunning, ...night } });
    await open(page);
    await expect(page.getByText("1:02:05 / 8:00:00")).toBeVisible();
    await expect(page.getByText(tk("ui.stab.night_running"))).toBeVisible();
    await emit(page, "stab-done", { ...stabDone, ...night, outcome: "errors", mismatches: 1 });
    await expect(
      page.getByText(tk("ui.stab.night_stopped", { time: "1:02:05", planned: tk("ui.stab.hours", { n: 8 }) })),
    ).toBeVisible();
  });

  test("збережений тест показує стан ПК і що змінилося від попереднього", async ({ page }) => {
    await mockBackend(page, {
      stab_list: [
        {
          ...stabDone,
          kind: "ram",
          planned_s: 28800,
          state: {
            optimax: "0.1.6",
            windows: null,
            windows_build: null,
            gpu: null,
            driver: null,
            display: null,
            power_plan: null,
            bios: "P2.40",
            ram_mts: 3600,
            hags: null,
            game_mode: null,
            game_file: null,
          },
          changes: [{ field: "ram", from: "3733 MT/s", to: "3600 MT/s" }],
        },
      ],
    });
    await open(page);
    await expect(page.getByText(tk("ui.stab.night_badge"), { exact: true })).toBeVisible();
    await expect(
      page.getByText(`${tk("ui.stab.state.bios", { v: "P2.40" })} · ${tk("ui.stab.state.ram", { v: 3600 })}`),
    ).toBeVisible();
    await expect(page.getByText(`${tk("ui.history.field.ram")}: 3733 MT/s → 3600 MT/s`)).toBeVisible();
  });

  test("збережені результати видно й можна видалити", async ({ page }) => {
    await mockBackend(page, { stab_list: [{ ...stabDone, outcome: "interrupted", state: null, changes: [] }] });
    await open(page);
    await expect(page.getByText(tk("ui.stab.saved_title"))).toBeVisible();
    await expect(page.getByText(tk("ui.stab.outcome.interrupted.title"))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.stab.delete") }).click();
    await expect.poll(async () => (await calls(page)).some((c) => c.cmd === "stab_delete")).toBe(true);
  });
});
