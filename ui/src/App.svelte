<script lang="ts">
  import { onMount } from "svelte";
  import { on } from "./lib/api";
  import type { DotsStats } from "./lib/dots";
  import { gm, pushLog, ui } from "./lib/state.svelte";
  import BackgroundDots from "./components/BackgroundDots.svelte";
  import LogPanel from "./components/LogPanel.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import Analysis from "./screens/Analysis.svelte";
  import Backups from "./screens/Backups.svelte";
  import Diagnosis from "./screens/Diagnosis.svelte";
  import Games from "./screens/Games.svelte";
  import Hardware from "./screens/Hardware.svelte";
  import Optimization from "./screens/Optimization.svelte";
  import Placeholder from "./screens/Placeholder.svelte";
  import Settings from "./screens/Settings.svelte";

  let dotsStats = $state<DotsStats | null>(null);

  onMount(() => {
    const subs = [
      on("log", pushLog),
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
    return () => subs.forEach((p) => void p.then((u) => u()));
  });

  // Фон — лише на «Аналізі» й «Діагностиці»; вимикається в режимі гри, під час вимірювань,
  // в економному режимі й коли користувач вимкнув його в налаштуваннях.
  const showBg = $derived(ui.screen === "analysis" || ui.screen === "diagnosis");
  const animate = $derived(
    showBg && ui.settings.animation !== "off" && !ui.settings.economy && !ui.gameActive && !ui.measuring,
  );
</script>

<div class="shell">
  {#if showBg}
    <BackgroundDots
      enabled={animate}
      quality={ui.settings.animation === "low" ? "low" : "high"}
      mode={ui.bgMode}
      leftInset={225}
      onstats={(s) => (dotsStats = s)}
    />
  {/if}
  <Sidebar />
  <main class="main">
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
        {:else if ui.screen === "backups"}
          <Backups />
        {:else if ui.screen === "settings"}
          <Settings {dotsStats} />
        {:else}
          <Placeholder />
        {/if}
      {/key}
    </div>
    <LogPanel />
  </main>
</div>

<style>
  .shell {
    display: flex;
    height: 100%;
    min-width: 960px;
    min-height: 640px;
  }
  .main {
    position: relative;
    z-index: 1;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .screen {
    flex: 1;
    min-height: 0;
  }
</style>
