<script lang="ts">
  import type { LatencyStat } from "../lib/bindings/LatencyStat";
  import type { LatencySummary } from "../lib/bindings/LatencySummary";
  import { tk } from "../lib/i18n";

  // Виміри затримки за запис. Усі значення рахує бекенд (`optimax_diagnosis::latency`).
  let { latency, frameGen = false }: { latency: LatencySummary; frameGen?: boolean } = $props();

  const ms = (v: number | null) => (v != null ? v.toFixed(1) : "—");
  const parts = $derived([
    { label: tk("ui.lat.cpu_busy"), value: latency.cpu_busy_ms },
    { label: tk("ui.lat.gpu_start"), value: latency.gpu_start_ms },
    { label: tk("ui.lat.until_displayed"), value: latency.until_displayed_ms },
  ]);
</script>

{#snippet tile(label: string, s: LatencyStat | null, none: string)}
  <div class="tile">
    {#if s}
      <strong class="mono">{tk("ui.lat.ms", { v: ms(s.median_ms) })}</strong>
      <span class="small">{label}</span>
      <span class="small muted mono">{tk("ui.lat.detail", { p95: ms(s.p95_ms), n: s.samples })}</span>
    {:else}
      <strong class="mono dim">—</strong>
      <span class="small">{label}</span>
      <span class="small muted">{none}</span>
    {/if}
  </div>
{/snippet}

<div class="lat">
  <div class="tiles">
    {@render tile(tk("ui.lat.frame_to_display"), latency.frame_to_display, tk("ui.lat.none_frames"))}
    {@render tile(tk("ui.lat.input_to_photon"), latency.input_to_photon, tk("ui.lat.none_input"))}
    {@render tile(tk("ui.lat.click_to_photon"), latency.click_to_photon, tk("ui.lat.none_click"))}
    {@render tile(tk("ui.lat.pc_latency"), latency.pc_latency, tk("ui.lat.none_pcl"))}
  </div>
  <dl class="parts">
    {#each parts as p (p.label)}
      <div>
        <dt class="small muted">{p.label}</dt>
        <dd class="mono">{tk("ui.lat.ms", { v: ms(p.value) })}</dd>
      </div>
    {/each}
  </dl>
  <p class="small muted">{tk("ui.lat.honest")}</p>
  {#if frameGen}<p class="small warn">{tk("ui.lat.frame_gen")}</p>{/if}
</div>

<style>
  .lat {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
    gap: calc(var(--space) * 3);
  }
  .tile {
    display: grid;
    gap: var(--space);
    align-content: start;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  .tile strong {
    font-size: 20px;
  }
  .parts {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 6);
    margin: 0;
  }
  .parts dt {
    margin-bottom: var(--space);
  }
  .parts dd {
    margin: 0;
  }
  .warn {
    color: var(--warn);
  }
  p {
    margin: 0;
  }
</style>
