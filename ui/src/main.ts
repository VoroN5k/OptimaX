// Шрифти вбудовано локально (OFL 1.1, без CDN): лише кирилиця й латиниця потрібних накреслень.
// Inter Tight — інтерфейс (300–700), JetBrains Mono — числа й клавіші (400, 500).
import "@fontsource/inter-tight/cyrillic-300.css";
import "@fontsource/inter-tight/cyrillic-400.css";
import "@fontsource/inter-tight/cyrillic-500.css";
import "@fontsource/inter-tight/cyrillic-600.css";
import "@fontsource/inter-tight/cyrillic-700.css";
import "@fontsource/inter-tight/latin-300.css";
import "@fontsource/inter-tight/latin-400.css";
import "@fontsource/inter-tight/latin-500.css";
import "@fontsource/inter-tight/latin-600.css";
import "@fontsource/inter-tight/latin-700.css";
import "@fontsource/jetbrains-mono/cyrillic-400.css";
import "@fontsource/jetbrains-mono/cyrillic-500.css";
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-500.css";
import "./styles/tokens.css";
import "./styles/base.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { api } from "./lib/api";
import { setCatalog, tk } from "./lib/i18n";
import { startSplash } from "./lib/splash";
import { logLocal, ui } from "./lib/state.svelte";

async function boot(): Promise<void> {
  const target = document.getElementById("app");
  if (!target) return;
  // Заставка грає паралельно із завантаженням даних бекенду, а не після нього.
  const splash = startSplash();
  try {
    // Каталог потрібен до першого рендеру, інакше користувач побачить сирі ключі.
    setCatalog(await api.catalog());
    const [info, settings, startup] = await Promise.all([api.appInfo(), api.getSettings(), api.startupReport()]);
    ui.info = info;
    ui.settings = settings;
    if (!info.elevated && !info.service) logLocal("warn", tk("ui.status.no_admin_log"));
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
    splash?.cancel();
    return;
  }
  mount(App, { target });
  splash?.ready();
}

void boot();
