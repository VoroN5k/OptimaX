<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { SensorsStatus } from "../lib/bindings/SensorsStatus";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import Modal from "./Modal.svelte";
  import StateView from "./StateView.svelte";
  import Toggle from "./Toggle.svelte";

  // Розширені датчики (PawnIO): драйвер ставить сам користувач, читання — лише після згоди тут.
  // Що показувати й чи підтримується процесор — вирішує бекенд.
  let { onsave }: { onsave: () => Promise<void> } = $props();

  let status = $state<SensorsStatus | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let consent = $state(false);
  /** Поки показання йдуть, оновлюємо їх рідко — це лише перевірка, що датчики працюють. */
  const POLL_MS = 3000;

  async function load() {
    loading = true;
    try {
      status = await api.sensorsStatus();
      error = null;
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
    const t = setInterval(() => {
      if (status?.state === "ok" && !document.hidden) void load();
    }, POLL_MS);
    return () => clearInterval(t);
  });

  async function toggle(on: boolean) {
    if (on) {
      // Увімкнення — лише через вікно згоди.
      ui.settings.pawnio_sensors = false;
      consent = true;
      return;
    }
    await onsave();
    await load();
  }

  async function accept() {
    consent = false;
    ui.settings.pawnio_sensors = true;
    await onsave();
    await load();
  }

  const c = (v: number) => `${v.toFixed(1)} °C`;
  const mhz = (v: number) => `${Math.round(v)}`;
</script>

<Card title={tk("ui.sensors.title")} subtitle={tk("ui.sensors.subtitle")}>
  <div class="rows">
    {#if status === null && loading}
      <StateView kind="loading" title={tk("ui.sensors.loading")} />
    {:else if error}
      <StateView kind="error" title={tk("ui.common.error")} text={error} />
    {:else if status?.state === "not_installed"}
      <p class="small">{tk("ui.sensors.not_installed")}</p>
      <div class="actions">
        <Button variant="secondary" icon="refresh" {loading} onclick={load}>{tk("ui.sensors.recheck")}</Button>
        <Button variant="primary" icon="folder" onclick={() => api.sensorsOpenSite()}>{tk("ui.sensors.open_site")}</Button>
      </div>
    {:else if status}
      <div class="row">
        <div>
          <strong>{tk("ui.sensors.enable")}</strong>
          <p class="small muted">{tk("ui.sensors.installed", { v: status.version })}</p>
        </div>
        <Toggle label={tk("ui.sensors.enable")} bind:checked={ui.settings.pawnio_sensors} onchange={(v) => toggle(v)} />
      </div>
      <div aria-live="polite">
        {#if status.state === "disabled"}
          <p class="small muted">{tk("ui.sensors.disabled")}</p>
        {:else if status.state === "unsupported"}
          <p class="small warn">{tk("ui.sensors.unsupported", { cpu: status.cpu })}</p>
        {:else if status.state === "failed"}
          <p class="small warn">{tk("ui.sensors.failed", { error: status.error })}</p>
        {:else if status.state === "ok"}
          {@const r = status.reading}
          <dl class="grid">
            <div><dt class="small muted">{tk("ui.sensors.cpu_temp")}</dt><dd class="mono">{c(r.cpu_c)}</dd></div>
            {#each r.ccd_c as t, i (i)}
              <div><dt class="small muted">{tk("ui.sensors.ccd_temp", { n: i + 1 })}</dt><dd class="mono">{c(t)}</dd></div>
            {/each}
            {#if r.clocks}
              <div>
                <dt class="small muted">{tk("ui.sensors.clocks")}</dt>
                <dd class="mono">{mhz(r.clocks.fclk_mhz)} / {mhz(r.clocks.uclk_mhz)} / {mhz(r.clocks.mclk_mhz)}</dd>
              </div>
            {/if}
          </dl>
          {#if !r.clocks}
            <p class="small dim">
              {r.pm_version != null
                ? tk("ui.sensors.clocks_unknown", { v: r.pm_version.toString(16).toUpperCase() })
                : tk("ui.sensors.clocks_none")}
            </p>
          {/if}
        {/if}
      </div>
      <p class="small dim">{tk("ui.sensors.note")}</p>
    {/if}
  </div>
</Card>

{#if consent}
  <Modal title={tk("ui.sensors.consent.title")} confirmLabel={tk("ui.sensors.consent.ok")} onconfirm={accept} oncancel={() => (consent = false)}>
    <p>{tk("ui.sensors.consent.what")}</p>
    <p>{tk("ui.sensors.consent.risk")}</p>
    <p>{tk("ui.sensors.consent.undo")}</p>
  </Modal>
{/if}

<style>
  .rows {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 4);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
    gap: calc(var(--space) * 4);
    margin: 0;
  }
  dt {
    margin-bottom: var(--space);
  }
  dd {
    margin: 0;
  }
  p {
    margin: 0;
  }
  .warn {
    color: var(--warn);
  }
</style>
