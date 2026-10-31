<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { ProcessEntry } from "../lib/bindings/ProcessEntry";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";

  // Вибір exe гри: зі списку запущених (найважчі зверху) або вручну.
  let { value = $bindable(""), label }: { value?: string; label: string } = $props();

  /** Як часто оновлювати список, поки сторінка відкрита. */
  const POLL_MS = 2000;

  let list = $state<ProcessEntry[]>([]);
  let loading = $state(false);
  /** Курсор над кнопками процесів: не переставляємо їх під мишею. */
  let hovering = $state(false);
  let busy = false;
  const id = `pp-${Math.random().toString(36).slice(2, 8)}`;

  async function refresh(manual = false) {
    if (busy) return;
    busy = true;
    if (manual) loading = true;
    try {
      list = await api.listProcesses();
    } catch {
      // Тимчасова помилка — лишаємо попередній список, наступна спроба за POLL_MS.
    } finally {
      busy = false;
      loading = false;
    }
  }

  onMount(() => {
    void refresh(true);
    // Автооновлення лише коли це має сенс: вікно видиме, не йде запис кадрів / A/B
    // (програма не має навантажувати систему під час замірів), курсор не над кнопками.
    const timer = setInterval(() => {
      if (document.hidden || ui.measuring || hovering) return;
      void refresh();
    }, POLL_MS);
    return () => clearInterval(timer);
  });
</script>

<div class="picker">
  <label for={id} class="small muted">{label}</label>
  <div class="row">
    <input
      {id}
      list="{id}-list"
      bind:value
      placeholder={tk("ui.picker.placeholder")}
      autocomplete="off"
      spellcheck="false"
    />
    <datalist id="{id}-list">
      {#each list as p (p.name)}
        <option value={p.name}>{p.name} — {p.cpu_percent.toFixed(1)}% CPU, {p.memory_mib} МіБ</option>
      {/each}
    </datalist>
    <Button small icon="refresh" label={tk("ui.picker.refresh")} {loading} onclick={() => refresh(true)} />
  </div>
  {#if list.length}
    <div
      class="chips"
      role="group"
      aria-label={tk("ui.picker.top")}
      onpointerenter={() => (hovering = true)}
      onpointerleave={() => (hovering = false)}
    >
      {#each list.slice(0, 6) as p (p.name)}
        <button type="button" class="chip" class:selected={p.name === value} onclick={() => (value = p.name)}>
          {p.name} <span class="dim">{p.cpu_percent.toFixed(1)}%</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .picker {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .row {
    display: flex;
    gap: calc(var(--space) * 2);
  }
  input {
    flex: 1;
    height: 38px;
    padding: 0 calc(var(--space) * 3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    font-family: var(--font-mono);
    font-size: 13px;
  }
  input:focus-visible {
    border-color: var(--accent-from);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 1.5);
  }
  .chip {
    padding: 3px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    font-size: 12px;
    cursor: pointer;
    transition: border-color var(--dur) var(--ease);
  }
  .chip:hover,
  .chip.selected {
    border-color: var(--accent-from);
  }
</style>
