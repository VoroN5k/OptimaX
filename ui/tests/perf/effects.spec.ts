import { chromium, test } from "@playwright/test";
import { gpuResult, settings } from "../e2e/fixtures";
import { mockBackend } from "../e2e/mock";

// Chromium з апаратним GPU — той самий рушій, що й WebView2; бекенд підроблено, умови однакові.
// CPU — частка одного ядра за процесами (CDP SystemInfo.getProcessInfo), пам'ять — JS-купа.
const SECONDS = Number(process.env.SECONDS_PER_MODE ?? 15);
const SCENARIOS = [
  { name: "Повні + фон із запису", effects: "full", background: "real", last: gpuResult },
  { name: "Повні + декоративний фон", effects: "full", background: "decorative", last: gpuResult },
  { name: "Повні без фону", effects: "full", background: "off", last: gpuResult },
  { name: "Спокійні", effects: "calm", background: "real", last: gpuResult },
  { name: "Вимкнені", effects: "off", background: "real", last: gpuResult },
  { name: "Повні, вікно неактивне (поверх гра)", effects: "full", background: "real", last: gpuResult, blur: true },
] as const;

test("витрати за режимами ефектів", async ({ baseURL }) => {
  const browser = await chromium.launch({
    headless: false,
    args: ["--ignore-gpu-blocklist", "--window-size=1280,860"],
  });
  const cdp = await browser.newBrowserCDPSession();
  const cpu = async () => {
    const { processInfo } = (await cdp.send("SystemInfo.getProcessInfo")) as {
      processInfo: { type: string; cpuTime: number }[];
    };
    const out: Record<string, number> = {};
    for (const p of processInfo) out[p.type] = (out[p.type] ?? 0) + p.cpuTime;
    return out;
  };
  const rows: Record<string, string>[] = [];
  for (const s of SCENARIOS) {
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, reducedMotion: "no-preference", locale: "uk-UA" });
    const page = await ctx.newPage();
    await mockBackend(page, {
      get_settings: { ...settings, effects: s.effects, background: s.background, onboarded: true },
      diag_state: { recording: false, last: s.last },
    });
    await page.addInitScript(() => localStorage.setItem("optimax.splash", "0"));
    await page.goto(baseURL ?? "/");
    await page.getByRole("button", { name: "Діагностика", exact: true }).click();
    if ("blur" in s) await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await page.waitForTimeout(3000);
    const a = await cpu();
    const t0 = Date.now();
    await page.waitForTimeout(SECONDS * 1000);
    const b = await cpu();
    const dt = (Date.now() - t0) / 1000;
    const heap = await page.evaluate(() => (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? 0);
    const pct = (k: string) => (100 * ((b[k] ?? 0) - (a[k] ?? 0))) / dt;
    const total = Object.keys(b).reduce((sum, k) => sum + pct(k), 0);
    rows.push({
      mode: s.name,
      total: total.toFixed(1),
      renderer: pct("renderer").toFixed(1),
      gpu: pct("GPU").toFixed(1) + "/" + pct("gpu").toFixed(1),
      browser: pct("browser").toFixed(1),
      heapMB: (heap / 1048576).toFixed(1),
    });
    await ctx.close();
  }
  await browser.close();
  console.log(JSON.stringify(rows, null, 1));
});
