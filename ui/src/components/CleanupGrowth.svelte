<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { GrowthView } from "../lib/bindings/GrowthView";
  import { date, size } from "../lib/format";
  import { tk } from "../lib/i18n";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import StateView from "./StateView.svelte";
  import VerdictBadge from "./VerdictBadge.svelte";

  // «Що виросло»: джерело проблеми, а не лише наслідок. Знімки — після кожного скану.
  let { drive, onreveal }: { drive: string; onreveal: (path: string) => void } = $props();

  let view = $state<GrowthView | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      view = drive ? await api.cleanupGrowth(drive) : null;
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  });

  const days = $derived(view ? Math.max(0, Math.round((view.now_unix - view.since_unix) / 86400)) : 0);
</script>

{#if loading}
  <Card><StateView kind="loading" title={tk("ui.common.loading")} /></Card>
{:else if error}
  <Card><StateView kind="error" title={tk("ui.common.error")} text={error} /></Card>
{:else if !view}
  <Card><StateView kind="empty" title={tk("ui.clean.growth_none_title")} text={tk("ui.clean.growth_none_text")} /></Card>
{:else}
  <Card
    title={tk("ui.clean.growth_title")}
    subtitle={tk("ui.clean.growth_period", { from: date(view.since_unix), to: date(view.now_unix), days })}
  >
    {#if days < 6}<p class="small warn">{tk("ui.clean.growth_short")}</p>{/if}
    {#if !view.full}<p class="small muted">{tk("ui.clean.growth_partial")}</p>{/if}
    {#if view.items.length}
      <div class="list">
        {#each view.items as g (g.growth.path)}
          <div class="row">
            <span class="mono path" title={g.growth.path}>{g.growth.path}</span>
            <span class="mono delta">+{size(g.growth.delta)}</span>
            <span class="mono small muted">
              {g.growth.before == null ? tk("ui.clean.growth_new") : size(g.growth.before)} → {size(g.growth.after)}
            </span>
            <VerdictBadge verdict={g.verdict} />
            <Button small variant="ghost" icon="folder" label={tk("ui.clean.reveal")} onclick={() => onreveal(g.growth.path)} />
          </div>
        {/each}
      </div>
    {:else}
      <p class="muted">{tk("ui.clean.growth_nothing")}</p>
    {/if}
  </Card>
{/if}

<style>
  .list {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 2) calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--bg-input);
    min-width: 0;
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .delta {
    color: var(--warn);
    font-weight: 600;
  }
  .warn {
    color: var(--warn);
  }
  p {
    margin: 0 0 calc(var(--space) * 3);
  }
</style>
