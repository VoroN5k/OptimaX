// UI-тести: інтерфейс у Chromium поверх Vite, бекенд підроблено (tests/e2e/mock.ts).
// `npm run test:ui`; перший раз — `npx playwright install chromium`.
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:1420",
    // Без заставки й анімацій: швидше й перевіряє шлях prefers-reduced-motion.
    reducedMotion: "reduce",
    locale: "uk-UA",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } } }],
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
