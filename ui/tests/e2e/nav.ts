// Перехід до розділу через нову навігацію: вкладка верхньої панелі, потім (якщо розділів у
// вкладці кілька) підвкладка в шапці сторінки. Відповідність — як у ui/src/lib/nav.ts.
import type { Page } from "@playwright/test";
import { tk } from "./text";

const TAB: Record<string, string> = {
  analysis: "overview",
  hardware: "overview",
  laptop: "overview",
  network: "overview",
  diagnosis: "diagnosis",
  stability: "tests",
  optimization: "tools",
  games: "tools",
  cleanup: "tools",
  startup: "tools",
  upscale: "tools",
  backups: "system",
  settings: "system",
};
const MULTI = new Set(["overview", "tools", "system"]);

/** Відкриває розділ `screen` (id з `ScreenId`). */
export async function go(page: Page, screen: string): Promise<void> {
  const tab = TAB[screen];
  if (!tab) throw new Error(`невідомий розділ ${screen}`);
  await page
    .getByRole("navigation", { name: tk("ui.nav.label") })
    .getByRole("button", { name: tk(`ui.tab.${tab}`), exact: true })
    .click();
  if (MULTI.has(tab)) await page.getByRole("tab", { name: tk(`ui.nav.${screen}`), exact: true }).click();
}
