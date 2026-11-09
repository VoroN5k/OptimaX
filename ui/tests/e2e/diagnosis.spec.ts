import { expect, test, type Page } from "@playwright/test";
import { abDone, expDone, gpuResult, resMid } from "./fixtures";
import { calls, emit, mockBackend } from "./mock";
import { tk } from "./text";

async function openDiagnosis(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: tk("ui.nav.diagnosis") }).click();
  await expect(page.getByRole("heading", { level: 1, name: tk("ui.diag.title") })).toBeVisible();
}

test.describe("Діагностика", () => {
  test("результат: вердикт зверху, знахідки компактні, без позначки прикладу", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await openDiagnosis(page);
    await expect(page.getByText(tk("diag.summary.gpu.title", { game: "re9.exe" }))).toBeVisible();
    await expect(page.locator("[data-tour=diag-stats]")).toContainText("125.0");
    await expect(page.getByText(tk("ui.diag.findings_n", { n: 2 }))).toBeVisible();
    // Перша знахідка розгорнута (видно пораду), друга — лише заголовок.
    await expect(page.getByText(tk("diag.gpu_bound.advice"), { exact: false })).toBeVisible();
    await expect(page.getByText(tk("diag.upscale.helps.detail"))).toHaveCount(0);
    await page.getByRole("button", { name: tk("diag.upscale.helps.title") }).click();
    await expect(page.getByText(tk("diag.upscale.helps.detail"))).toBeVisible();
    await expect(page.getByText(tk("ui.tour.example_title"))).toHaveCount(0);
    // Налаштування запису згорнуті в рядок, тест роздільності — згорнутий блок.
    await expect(page.locator("[data-tour=diag-game]")).toHaveCount(0);
    await expect(page.getByRole("button", { name: tk("ui.res.title") })).toHaveAttribute("aria-expanded", "false");
  });

  test("запис: команда старту, хід і результат з подій бекенду", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await openDiagnosis(page);
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "diag_start")?.args)
      .toEqual({ exe: "re9.exe", seconds: 60, scene: null });
    await emit(page, "diag-progress", { phase: "recording", counted_s: 12, target_s: 60, elapsed_s: 14 });
    await expect(page.getByRole("button", { name: tk("ui.diag.stop") })).toBeVisible();
    const done = structuredClone(gpuResult);
    done.diagnosis.verdicts = done.diagnosis.verdicts.slice(0, 1);
    await emit(page, "diag-done", done);
    await expect(page.getByText(tk("ui.diag.no_more"))).toBeVisible();
  });

  test("мітка сцени передається в запис", async ({ page }) => {
    await mockBackend(page);
    await openDiagnosis(page);
    await page.getByLabel(tk("ui.diag.scene")).fill("  коридор ");
    await page.getByRole("button", { name: tk("ui.picker.refresh") }).waitFor();
    await page.locator("[data-tour=diag-game] input").fill("re9.exe");
    await page.getByRole("button", { name: tk("ui.diag.start") }).click();
    await expect
      .poll(async () => (await calls(page)).find((c) => c.cmd === "diag_start")?.args)
      .toEqual({ exe: "re9.exe", seconds: 60, scene: "коридор" });
  });

  test("«Довідка» пояснює вердикти, Esc закриває", async ({ page }) => {
    await mockBackend(page);
    await openDiagnosis(page);
    await page.getByRole("button", { name: tk("ui.guide.open") }).click();
    const dialog = page.getByRole("dialog", { name: tk("ui.guide.title") });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(tk("ui.guide.v.cpu.title"));
    await expect(dialog).toContainText(tk("ui.guide.t.low1.text"));
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  });
  test("анатомія кадру: типовий кадр, черга і що розтягнуло ривки", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: structuredClone(gpuResult) } });
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.nav.diagnosis") }).click();
    const fold = page.getByRole("button", { name: new RegExp(tk("ui.anatomy.title")) });
    await expect(fold).toContainText(tk("ui.anatomy.summary", { ms: "8.0", n: 1 }));
    await fold.click();
    await expect(page.getByText(tk("ui.anatomy.queued", { pct: 34 }))).toBeVisible();
    await expect(page.getByText(`${tk("ui.anatomy.stall.cpu_busy")} · 1`)).toBeVisible();
    await expect(page.locator("table.spikes tbody tr")).toHaveCount(1);
    await expect(page.locator("table.spikes tbody tr")).toContainText("55.2");
    await expect(page.locator("table.spikes tbody tr")).toContainText(`${tk("ui.anatomy.main.page_in")} 36.0`);
    await expect(page.getByText(`${tk("ui.anatomy.main.page_in")} · 1`)).toBeVisible();
  });

  test("шкала запису: перехід до найгіршого ривку й значення під курсором", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: structuredClone(gpuResult) } });
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.nav.diagnosis") }).click();
    const fold = page.getByRole("button", { name: new RegExp(tk("ui.tl.title")) });
    await expect(fold).toContainText(tk("ui.tl.summary", { s: "30", n: 1 }));
    await fold.click();
    const canvas = page.getByRole("application", { name: tk("ui.tl.aria") });
    await expect(canvas).toBeVisible();
    await page.getByRole("button", { name: tk("ui.tl.worst") }).click();
    await expect(page.getByText(tk("ui.tl.spike", { i: 1, n: 1, t: "11.56", ms: "60.0" }))).toBeVisible();
    // Відрізок ривку займає видимий діапазон: курсор посередині — у ривку, головний потік чекав на диск.
    await canvas.scrollIntoViewIfNeeded();
    const box = await canvas.boundingBox();
    if (!box) throw new Error("немає полотна");
    const at = (ms: number) => {
      // Видимо [12564 - 60 - 400, 12564 + 300]; підписи ліворуч — 104 px.
      const from = 12104;
      const to = 12864;
      return box.x + 104 + ((ms - from) / (to - from)) * (box.width - 104);
    };
    await page.mouse.move(at(12528.6), box.y + 20);
    await expect(page.locator(".readout")).toContainText(tk("ui.anatomy.main.page_in"));
    await expect(page.locator(".readout")).toContainText("nvlddmkm.sys: 1.20");
    await expect(page.locator(".readout")).toContainText(tk("ui.tl.frame", { ft: "60.0", cpu: "42.0", gpu: "7.4", q: "3.2" }));
    // Температура CPU (з PawnIO) — у тій самій підказці.
    await expect(page.locator(".readout")).toContainText(tk("ui.tl.cpu_temp", { c: "73" }));
  });

  test("порівняння: старт з назвами, підказка варіанта, висновок з інтервалом", async ({ page }) => {
    await mockBackend(page, {
      exp_list: [
        {
          id: "1793600000000-re9.exe",
          game: "re9.exe",
          at_unix_ms: 1793600000000,
          names: ["RT вимк.", "RT увімк."],
          seconds: 30,
          report: structuredClone(expDone.report),
        },
      ],
    });
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.nav.diagnosis") }).click();
    await page.getByRole("radio", { name: tk("ui.exp.tab") }).click();
    // Збережене порівняння видно до старту нового.
    await expect(page.getByText(tk("ui.exp.saved_title"))).toBeVisible();
    await page.getByRole("combobox", { name: tk("ui.diag.game") }).fill("re9.exe");
    await page.getByLabel(tk("ui.exp.name_a")).fill("DLSS Якість");
    await page.getByLabel(tk("ui.exp.name_n", { l: "B" }), { exact: true }).fill("DLSS Баланс");
    // Третій варіант можна додати й прибрати.
    await page.getByRole("button", { name: tk("ui.exp.add_variant", { l: "C" }) }).click();
    await page.getByLabel(tk("ui.exp.name_n", { l: "C" }), { exact: true }).fill("DLSS Продуктивність");
    await expect(page.getByText(tk("ui.exp.how_many", { n: 3 }))).toBeVisible();
    await page.getByRole("button", { name: tk("ui.exp.remove_variant", { l: "C" }) }).click();
    await expect(page.getByLabel(tk("ui.exp.name_n", { l: "C" }), { exact: true })).toHaveCount(0);
    await page.getByRole("switch", { name: tk("ui.exp.warmup") }).click();
    await page.getByRole("button", { name: tk("ui.exp.start") }).click();
    const start = (await calls(page)).find((c) => c.cmd === "exp_start");
    expect(start?.args).toMatchObject({ exe: "re9.exe", names: ["DLSS Якість", "DLSS Баланс"], seconds: 30, warmup: false });
    await expect(page.getByText(tk("ui.exp.keep", { name: "DLSS Якість", n: 1 }))).toBeVisible();

    await page.getByRole("button", { name: tk("ui.exp.run", { n: 1 }) }).click();
    await expect(
      page.getByText(
        tk("ui.exp.verdict.better", { a: "DLSS Якість", b: "DLSS Баланс", d: "+6.0", lo: "+3.1", hi: "+8.9", low: "+4.1" }),
      ).first(),
    ).toBeVisible();
    await expect(page.locator("table.runs tbody tr")).toHaveCount(6);
    await expect(page.getByText(tk("ui.exp.suspect.level"))).toBeVisible();
    await expect(page.getByText(tk("ui.exp.outcome.unclear"))).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.exp.close") })).toBeVisible();
  });

  test("тест роздільності: підказка повернути роздільність і поточний інтервал", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult }, res_state: resMid });
    await openDiagnosis(page);
    await expect(page.getByText(tk("ui.res.switch_back", { n: 4 }))).toBeVisible();
    await expect(page.getByText(tk("ui.res.now", { delta: "+37.5", lo: "+20.1", hi: "+54.9" }), { exact: false })).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.res.run", { n: 4 }) })).toBeVisible();
  });

  test("A/B оптимізації: прогони з рушія, інтервал і підсумок без приросту", async ({ page }) => {
    await mockBackend(page, { ab_state: abDone });
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.nav.optimization") }).click();
    await expect(page.locator("ol.runs li")).toHaveCount(6);
    await expect(page.getByText(tk("ui.opt.done.no_effect"))).toBeVisible();
    await expect(page.getByText(tk("ui.exp.outcome.no_effect")).first()).toBeVisible();
    await page.getByRole("button", { name: tk("ui.opt.finish_revert") }).click();
    await expect.poll(async () => (await calls(page)).find((c) => c.cmd === "ab_finish")?.args).toEqual({ keep: false });
    await expect(page.getByText(tk("ui.opt.effect_none"), { exact: false })).toBeVisible();
  });

  test("затримка: виміри з кількістю кадрів і чесні «немає даних»", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await openDiagnosis(page);
    await page.getByRole("button", { name: tk("ui.lat.title") }).click();
    await expect(page.getByText(tk("ui.lat.summary", { ms: "18.4" }))).toBeVisible();
    await expect(page.getByText(tk("ui.lat.detail", { p95: "27.1", n: 3700 }))).toBeVisible();
    await expect(page.getByText(tk("ui.lat.none_pcl"))).toBeVisible();
    await expect(page.getByText(tk("ui.lat.honest"))).toBeVisible();
    // Ввід → екран занижує затримку: лише довідково, із застереженням.
    await expect(page.getByText(tk("ui.lat.ref_value", { v: "31.2", n: 812 }))).toBeVisible();
    await expect(page.getByText(tk("ui.lat.reference_note"))).toBeVisible();
  });

  test("затримка: PC Latency у заголовку і підказка про Reflex без обіцянки приросту", async ({ page }) => {
    const last = structuredClone(gpuResult);
    last.diagnosis.latency!.pc_latency = { median_ms: 47.1, p95_ms: 52.6, samples: 3700 };
    const hint = structuredClone(last.diagnosis.verdicts.at(-1)!);
    hint.finding.id = "latency_queue";
    hint.finding.confidence = 60;
    hint.finding.title = { key: "diag.latency_queue.title", args: { gap: "19.2", frames: "1.4" } };
    hint.finding.detail = { key: "diag.latency_queue.detail", args: { gap: "19.2", frames: "1.4" } };
    hint.finding.advice = { key: "diag.latency_queue.advice", args: {} };
    last.diagnosis.verdicts.push(hint);
    await mockBackend(page, { diag_state: { recording: false, last } });
    await openDiagnosis(page);
    await expect(page.getByText(tk("ui.lat.summary_pcl", { ms: "47.1" }))).toBeVisible();
    const title = tk("diag.latency_queue.title", { gap: "19.2", frames: "1.4" });
    await page.getByRole("button", { name: title }).click();
    await expect(page.getByText(tk("diag.latency_queue.advice"))).toBeVisible();
  });

  test("затримка: кнопка відкриває «Порівняння» з готовими варіантами, але не запускає його", async ({ page }) => {
    const last = structuredClone(gpuResult);
    last.diagnosis.latency!.pc_latency = { median_ms: 47.1, p95_ms: 52.6, samples: 3700 };
    await mockBackend(page, { diag_state: { recording: false, last } });
    await openDiagnosis(page);
    await page.getByRole("button", { name: tk("ui.lat.title") }).click();
    await page.getByRole("button", { name: tk("ui.lat.compare_reflex") }).click();
    await expect(page.getByLabel(tk("ui.exp.name_a"))).toHaveValue(tk("ui.exp.preset.reflex_off"));
    await expect(page.getByLabel(tk("ui.exp.name_n", { l: "B" }), { exact: true })).toHaveValue(tk("ui.exp.preset.reflex_on"));
    await expect(page.getByRole("combobox", { name: tk("ui.diag.game") })).toHaveValue(last.diagnosis.game);
    expect((await calls(page)).some((c) => c.cmd === "exp_start")).toBe(false);
  });

  test("затримка: без PC Latency кнопки порівняння немає", async ({ page }) => {
    await mockBackend(page, { diag_state: { recording: false, last: gpuResult } });
    await openDiagnosis(page);
    await page.getByRole("button", { name: tk("ui.lat.title") }).click();
    await expect(page.getByText(tk("ui.lat.none_pcl"))).toBeVisible();
    await expect(page.getByRole("button", { name: tk("ui.lat.compare_reflex") })).toHaveCount(0);
  });

  test("порівняння: PC Latency окремо від висновку про FPS, компроміс не фарбується як «гірше»", async ({ page }) => {
    const view = structuredClone(expDone);
    view.report.margin_latency_pct = 5;
    const c = view.report.comparisons[0]!;
    c.verdict = "worse";
    c.avg_fps = { ...c.avg_fps!, delta_pct: -1.7, ci_low_pct: -2.4, ci_high_pct: -1.0, outcome: "worse" };
    c.pc_latency = {
      a: 47.1,
      b: 32.9,
      delta_pct: -30.1,
      ci_low_pct: -33.0,
      ci_high_pct: -27.0,
      drift_pct_per_run: 0.1,
      run_sd_pct: 1.2,
      outcome: "better",
    };
    view.report.runs[0]!.pc_latency_ms = 47.2;
    await mockBackend(page, { exp_state: view });
    await page.goto("/");
    await page.getByRole("button", { name: tk("ui.nav.diagnosis") }).click();
    const line = tk("ui.exp.verdict_latency", {
      a: "47.1",
      b: "32.9",
      d: "-30.1",
      lo: "-33.0",
      hi: "-27.0",
      outcome: tk("ui.exp.outcome.better"),
    });
    await expect(page.getByText(line)).toBeVisible();
    await expect(page.locator(".box.done.worse")).toHaveCount(0);
    await expect(page.getByRole("columnheader", { name: tk("ui.exp.col.latency") })).toBeVisible();
    await expect(page.locator("table.runs tbody tr").first()).toContainText("47.2");
    await expect(page.getByText(tk("ui.exp.latency_note"))).toBeVisible();
  });
});
