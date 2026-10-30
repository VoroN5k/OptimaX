<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { StartupEntry } from "../lib/bindings/StartupEntry";
  import type { UiDesired } from "../lib/bindings/UiDesired";
  import { tk } from "../lib/i18n";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import Page from "../components/Page.svelte";
  import StateView from "../components/StateView.svelte";
  import Toggle from "../components/Toggle.svelte";
  import TweakApplyModal from "../components/TweakApplyModal.svelte";

  // Перелік і навантаження визначає бекенд; перемикач лише відкриває звичайний план твіку
  // «що зміниться» (журнал, відкат, точка відновлення).
  let entries = $state<StartupEntry[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let desired = $state<UiDesired | null>(null);

  async function load() {
    loading = true;
    error = null;
    try {
      entries = await api.startupList();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }
  onMount(load);

  function toggle(e: StartupEntry, enabled: boolean) {
    desired = { tweak: "startup_app", value: { source: e.source, name: e.name, enabled } };
  }

  async function closed() {
    desired = null;
    // Перечитуємо стан із системи: і після застосування, і після скасування (перемикач повернеться).
    await load();
  }

  const enabledCount = $derived(entries.filter((e) => e.enabled).length);
  const running = $derived(entries.filter((e) => e.running));
  const cpu = $derived(running.reduce((s, e) => s + (e.running?.cpu_percent ?? 0), 0));
  const mem = $derived(running.reduce((s, e) => s + Number(e.running?.memory_mib ?? 0), 0));
</script>

<Page title={tk("ui.startup.title")} subtitle={tk("ui.startup.subtitle")}>
  {#if error}
    <Card><StateView kind="error" title={tk("ui.startup.error")} text={error} /></Card>
  {:else if loading && !entries.length}
    <Card><StateView kind="loading" title={tk("ui.common.loading")} /></Card>
  {:else if !entries.length}
    <Card><StateView kind="empty" title={tk("ui.startup.empty_title")} text={tk("ui.startup.empty_text")} /></Card>
  {:else}
    <Card title={tk("ui.startup.summary_title")}>
      <p>{tk("ui.startup.summary", { on: enabledCount, total: entries.length, running: running.length, cpu: cpu.toFixed(1), mem })}</p>
      <p class="small muted note">{tk("ui.startup.honest")}</p>
    </Card>

    <Card padded={false}>
      <ul class="list" aria-busy={loading}>
        {#each entries as e (e.source + e.name)}
          <li class:off={!e.enabled}>
            <div class="main">
              <div class="top">
                <strong class="name" title={e.name}>{e.name}</strong>
                <span class="badge">{tk(`ui.startup.source.${e.source}`)}</span>
                {#if e.lock}
                  <span class="badge lock" title={tk(`ui.startup.lock.${e.lock}`)}>{tk(`ui.startup.lock_short.${e.lock}`)}</span>
                {/if}
              </div>
              <div class="mono small muted cmd" title={e.command}>{e.command}</div>
              <div class="small">
                {#if e.running}
                  <span class="run">{tk("ui.startup.running", { n: e.running.processes, cpu: e.running.cpu_percent.toFixed(1), mem: Number(e.running.memory_mib) })}</span>
                {:else if e.exe_exists === false}
                  <span class="warn">{tk("ui.startup.missing")}</span>
                {:else}
                  <span class="dim">{tk("ui.startup.not_running")}</span>
                {/if}
              </div>
              {#if e.lock}<p class="small muted">{tk(`ui.startup.lock.${e.lock}`)}</p>{/if}
            </div>
            <div class="switch">
              <span class="small muted">{e.enabled ? tk("tweak.startup_app.enabled") : tk("tweak.startup_app.disabled")}</span>
              <Toggle
                label={tk("ui.startup.toggle", { name: e.name })}
                checked={e.enabled}
                disabled={!!e.lock || loading}
                onchange={(v) => toggle(e, v)}
              />
            </div>
          </li>
        {/each}
      </ul>
    </Card>
  {/if}

  {#snippet secondary()}
    <span class="small muted">{tk("ui.startup.where_rollback")}</span>
  {/snippet}
  {#snippet primary()}
    <Button icon="undo" {loading} onclick={load}>{tk("ui.startup.refresh")}</Button>
  {/snippet}
</Page>

{#if desired}
  <TweakApplyModal {desired} onclose={closed} />
{/if}

<style>
  p {
    margin: 0;
  }
  .note {
    margin-top: calc(var(--space) * 2);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 4);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-top: 1px solid var(--border);
  }
  li:first-child {
    border-top: none;
  }
  li.off .name {
    color: var(--text-muted);
  }
  .main {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: calc(var(--space) * 1);
  }
  .top {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cmd {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    flex-shrink: 0;
    padding: 0 calc(var(--space) * 2);
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    font-size: 11px;
    line-height: 18px;
    color: var(--text-muted);
  }
  .badge.lock {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 50%, var(--border));
  }
  .run {
    color: var(--text);
  }
  .warn {
    color: var(--warn);
  }
  .switch {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    flex-shrink: 0;
  }
</style>
