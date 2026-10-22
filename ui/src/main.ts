// Шрифти вбудовано локально (OFL 1.1, без CDN): лише кирилиця й латиниця потрібних накреслень.
import "@fontsource/inter/cyrillic-400.css";
import "@fontsource/inter/cyrillic-500.css";
import "@fontsource/inter/cyrillic-600.css";
import "@fontsource/inter/latin-400.css";
import "@fontsource/inter/latin-500.css";
import "@fontsource/inter/latin-600.css";
import "@fontsource/jetbrains-mono/cyrillic-400.css";
import "@fontsource/jetbrains-mono/latin-400.css";
import "./styles/tokens.css";
import "./styles/base.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { api } from "./lib/api";
import { setCatalog, tk } from "./lib/i18n";
import { logLocal, ui } from "./lib/state.svelte";

async function boot(): Promise<void> {
  const target = document.getElementById("app");
  if (!target) return;
  try {
    // Каталог потрібен до першого рендеру, інакше користувач побачить сирі ключі.
    setCatalog(await api.catalog());
    const [info, settings, startup] = await Promise.all([api.appInfo(), api.getSettings(), api.startupReport()]);
    ui.info = info;
    ui.settings = settings;
    if (!info.elevated) logLocal("warn", tk("ui.status.no_admin_log"));
    if (startup.recovered_pending > 0 || startup.restored_gamemode.rolled_back.length > 0) {
      logLocal(
        "warn",
        tk("ui.startup.recovered", {
          n: startup.recovered_pending,
          g: startup.restored_gamemode.rolled_back.length,
        }),
      );
    }
    if (startup.error) logLocal("error", startup.error);
  } catch (e) {
    target.textContent = `OptimaX: ${String(e)}`;
    return;
  }
  mount(App, { target });
}

void boot();
