<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { DriverEffect } from "../lib/bindings/DriverEffect";
  import { tk } from "../lib/i18n";
  import { logLocal, ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";

  // Рядок «Драйвер» на «Головній»: перевірка новішого драйвера NVIDIA на nvidia.com (лише за
  // кнопкою або якщо користувач увімкнув перевірку при запуску) і ефект останнього оновлення
  // драйвера з історії записів (без мережі). Висновки рахує бекенд.

  let checking = $state(false);
  let error = $state<string | null>(null);
  let effects = $state<DriverEffect[]>([]);

  onMount(() => {
    void api
      .historyDriverEffects()
      .then((e) => (effects = e))
      .catch(() => (effects = []));
  });

  async function check() {
    checking = true;
    error = null;
    try {
      ui.driverCheck = await api.driverCheck();
    } catch (e) {
      error = asCmdError(e).message;
      logLocal("warn", tk("ui.driver_online.failed", { message: error }));
    } finally {
      checking = false;
    }
  }

  const c = $derived(ui.driverCheck);
  const pct = (v: number | null) => (v == null ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(0)} %`);

  function compare(e: DriverEffect) {
    ui.pendingCompare = [e.comparison.older.id, e.comparison.newer.id];
    ui.screen = "diagnosis";
  }
</script>

<div class="driver">
  <div class="line">
    <div class="text" aria-live="polite">
      {#if error}
        <span class="small error">{tk("ui.driver_online.failed", { message: error })}</span>
      {:else if !c}
        <span class="small muted">{tk("ui.driver_online.hint")}</span>
      {:else if c.kind === "not_nvidia"}
        <span class="small muted">{tk("ui.driver_online.not_nvidia")}</span>
      {:else if c.kind === "unknown_model"}
        <span class="small muted">{tk("ui.driver_online.unknown_model", { gpu: c.gpu })}</span>
      {:else if c.newer}
        <strong class="newer">
          {tk("ui.driver_online.newer", { v: c.latest, date: c.released ?? "—", current: c.installed ?? "?" })}
        </strong>
      {:else if c.installed}
        <span class="small ok">{tk("ui.driver_online.latest", { v: c.installed })}</span>
      {:else}
        <span class="small muted">{tk("ui.driver_online.site_version", { v: c.latest })}</span>
      {/if}
    </div>
    {#if c?.kind === "checked" && c.newer && c.page}
      {@const page = c.page}
      <Button small variant="secondary" icon="folder" onclick={() => api.driverOpenPage(page)}>
        {tk("ui.driver_online.open_page")}
      </Button>
    {/if}
    <Button small variant="ghost" icon="refresh" loading={checking} onclick={check}>
      {tk("ui.driver_online.check")}
    </Button>
  </div>

  {#if effects.length}
    <div class="effects">
      <span class="small muted">{tk("ui.driver_online.effect", { from: effects[0]?.from ?? "", to: effects[0]?.to ?? "" })}</span>
      {#each effects as e (e.comparison.newer.id)}
        <button type="button" class="chip small {e.comparison.trend}" onclick={() => compare(e)}>
          {e.comparison.newer.game}
          {e.comparison.trend === "same"
            ? tk("ui.driver_online.effect_same")
            : pct(e.comparison.avg_fps_change_pct)}
        </button>
      {/each}
      {#if effects.some((e) => e.comparison.changes.length > 1 || e.comparison.scene !== "same")}
        <span class="small dim">{tk("ui.driver_online.effect_note")}</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .driver {
    display: grid;
    gap: calc(var(--space) * 2);
    margin-top: calc(var(--space) * 3);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  .line,
  .effects {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .newer {
    color: var(--warn);
    font-size: 13px;
  }
  .ok {
    color: var(--ok);
  }
  .error {
    color: var(--danger);
  }
  .chip {
    padding: 2px calc(var(--space) * 2);
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    background: var(--bg-card);
    color: var(--text);
    font: inherit;
    cursor: pointer;
    transition: border-color var(--dur-fast) var(--ease);
  }
  .chip:hover {
    border-color: var(--accent-from);
  }
  .chip:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
  }
  .chip.better {
    color: var(--ok);
  }
  .chip.worse {
    color: var(--warn);
  }
</style>
