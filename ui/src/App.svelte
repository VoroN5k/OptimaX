<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "./lib/api";
  import { tk } from "./lib/i18n";
  import { buildBackdropData } from "./lib/backdrop";
  import { rememberSplash } from "./lib/splash";
  import { effectiveEffects, measuringNow, trackEffects } from "./lib/effects.svelte";
  import { cl, flashBg, gm, logLocal, pushLog, ui, up } from "./lib/state.svelte";
  import Backdrop from "./components/Backdrop.svelte";
  import LogPanel from "./components/LogPanel.svelte";
  import TopBar from "./components/TopBar.svelte";
  import { remember } from "./lib/nav";
  import Tour from "./components/Tour.svelte";
  import { startTour } from "./lib/tour.svelte";
  import UpdateBanner from "./components/UpdateBanner.svelte";
  import Analysis from "./screens/Analysis.svelte";
  import Backups from "./screens/Backups.svelte";
  import Cleanup from "./screens/Cleanup.svelte";
  import Diagnosis from "./screens/Diagnosis.svelte";
  import Games from "./screens/Games.svelte";
  import Hardware from "./screens/Hardware.svelte";
  import Laptop from "./screens/Laptop.svelte";
  import Network from "./screens/Network.svelte";
  import Optimization from "./screens/Optimization.svelte";
  import Placeholder from "./screens/Placeholder.svelte";
  import Settings from "./screens/Settings.svelte";
  import Startup from "./screens/Startup.svelte";
  import Upscale from "./screens/Upscale.svelte";
  import Stability from "./screens/Stability.svelte";

  let bgStats = $state<{ fps: number; ms: number } | null>(null);

  onMount(() => {
    const stopEffects = trackEffects();
    // Перевірка оновлень при запуску — лише якщо користувач сам її ввімкнув (мережевий запит).
    if (ui.settings.update_check_on_start) {
      void api
        .updateCheck()
        .then((s) => {
          ui.update = s;
          if (s.available) logLocal("info", tk("ui.update.available_log", { v: s.available.version }));
        })
        .catch((e) => logLocal("warn", tk("ui.update.check_failed", { message: asCmdError(e).message })));
    }
    // Перевірка драйвера NVIDIA при запуску — теж лише якщо користувач її ввімкнув.
    if (ui.settings.driver_check_on_start) {
      void api
        .driverCheck()
        .then((c) => {
          ui.driverCheck = c;
          if (c.kind === "checked" && c.newer) logLocal("info", tk("ui.driver_online.newer_log", { v: c.latest }));
        })
        .catch((e) => logLocal("warn", tk("ui.driver_online.failed", { message: asCmdError(e).message })));
    }
    // Останній запис — джерело фону з реальних даних.
    void api
      .diagState()
      .then((s) => {
        if (s.last && !ui.lastRecord) ui.lastRecord = s.last;
      })
      .catch(() => undefined);
    void api.upscaleState().then((v) => {
      up.view = v;
    });
    const subs = [
      on("log", pushLog),
      on("diag-done", (r) => (ui.lastRecord = r)),
      on("upscale", (v) => {
        up.view = v;
      }),
      on("cleanup-progress", (p) => (cl.progress = p)),
      // Проміжний звіт: знайдене видно ще до кінця скану (вибір заблоковано, поки скан триває).
      on("cleanup-scan-partial", (v) => {
        if (cl.scanning) cl.view = v;
      }),
      on("cleanup-scan-done", (v) => {
        cl.view = v;
        cl.scanning = false;
        cl.progress = null;
      }),
      on("cleanup-exec-progress", (p) => (cl.execProgress = p)),
      on("cleanup-exec-done", (v) => {
        cl.lastExec = v;
        cl.executing = false;
        cl.execProgress = null;
        flashBg("ok");
      }),
      on("cleanup-error", (e) => {
        cl.error = e.message;
        logLocal("error", e.message);
        cl.scanning = false;
        cl.executing = false;
        flashBg("warn");
      }),
      on("gamemode", (e) => {
        switch (e.kind) {
          case "watching":
            gm.watching = true;
            gm.games = e.games;
            break;
          case "started":
            gm.active = { game: e.game, pid: e.pid, applied: e.applied, skipped: e.skipped };
            ui.gameActive = true;
            break;
          case "needs_confirmation":
            gm.needsConfirmation = { game: e.game, reason: e.reason };
            break;
          case "ended":
            gm.active = null;
            gm.lastEnded = { game: e.game, result: e.result };
            ui.gameActive = false;
            break;
          case "stopped":
            gm.watching = false;
            gm.active = null;
            ui.gameActive = false;
            break;
        }
      }),
    ];
    // Перший запуск: знайомство після заставки (вона ховає вміст, поки грає).
    let tourTimer: ReturnType<typeof setTimeout> | undefined;
    if (!ui.settings.onboarded) {
      const wait = () => {
        if (document.documentElement.classList.contains("splashing")) tourTimer = setTimeout(wait, 200);
        else startTour();
      };
      wait();
    }
    return () => {
      clearTimeout(tourTimer);
      stopEffects();
      subs.forEach((p) => void p.then((u) => u()));
    };
  });

  // Заставка запуску вирішується до читання налаштувань — запам'ятовуємо їх локально.
  $effect(() => rememberSplash(effectiveEffects() === "full"));

  // Фоновий шар (CLAUDE.md 10.4): лише з «Ефектами: Повні», не під час вимірювань і режиму гри.
  const bgActive = $derived(
    effectiveEffects() === "full" && !measuringNow() && ui.settings.background !== "off",
  );
  const bgData = $derived(
    ui.settings.background === "real" ? buildBackdropData(ui.lastRecord?.diagnosis.view) : null,
  );

  // Повернення на вкладку відкриває той розділ, де користувач був.
  $effect(() => remember(ui.screen));

</script>

<div class="shell">
  <Backdrop
    active={bgActive}
    data={bgData}
    game={bgData ? (ui.lastRecord?.diagnosis.game ?? null) : null}
    grain={effectiveEffects() !== "off"}
    onstats={(s) => (bgStats = s)}
  />
  <TopBar />
  <main class="main">
    <UpdateBanner />
    <div class="screen">
      {#key ui.screen}
        {#if ui.screen === "analysis"}
          <Analysis />
        {:else if ui.screen === "diagnosis"}
          <Diagnosis />
        {:else if ui.screen === "optimization"}
          <Optimization />
        {:else if ui.screen === "games"}
          <Games />
        {:else if ui.screen === "hardware"}
          <Hardware />
        {:else if ui.screen === "laptop"}
          <Laptop />
        {:else if ui.screen === "network"}
          <Network />
        {:else if ui.screen === "cleanup"}
          <Cleanup />
        {:else if ui.screen === "startup"}
          <Startup />
        {:else if ui.screen === "upscale"}
          <Upscale />
        {:else if ui.screen === "stability"}
          <Stability />
        {:else if ui.screen === "backups"}
          <Backups />
        {:else if ui.screen === "settings"}
          <Settings {bgStats} />
        {:else}
          <Placeholder />
        {/if}
      {/key}
    </div>
    <LogPanel />
  </main>
</div>
<Tour />

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 960px;
    min-height: 640px;
  }
  .main {
    position: relative;
    z-index: 1;
    flex: 1;
    min-width: 0;
    /* Без цього колонка росте до висоти вмісту, і прокрутка сторінки (.scroll у Page) зникає. */
    min-height: 0;
    display: flex;
    flex-direction: column;
    transition:
      opacity var(--dur-slow) var(--ease),
      transform var(--dur-slow) var(--ease);
  }
  /* Під заставкою запуску контент прихований; з'являється fade + зсув на 8px. */
  :global(:root.splashing) .main {
    opacity: 0;
    transform: translateY(8px);
  }
  .screen {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
