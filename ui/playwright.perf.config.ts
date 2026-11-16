// Заміри витрат інтерфейсу за режимами «Ефектів» (етап 7 редизайну, docs/DESIGN.md розд. 10):
// production-збірка у вікні Chromium з апаратним GPU. Не входить у звичайні UI-тести.
//   npm run build; npx playwright test -c playwright.perf.config.ts
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests/perf",
  reporter: "list",
  timeout: 300_000,
  workers: 1,
  use: { baseURL: "http://127.0.0.1:4173", locale: "uk-UA" },
  webServer: {
    command: "npx vite preview --port 4173 --strictPort",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
