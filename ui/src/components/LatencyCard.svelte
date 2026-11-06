<script lang="ts">
  import type { LatencyStat } from "../lib/bindings/LatencyStat";
  import type { LatencySummary } from "../lib/bindings/LatencySummary";
  import { tk } from "../lib/i18n";
  import Button from "./Button.svelte";

  // Виміри затримки за запис. Усі значення рахує бекенд (`optimax_diagnosis::latency`).
  // `onCompareReflex` — відкрити «Порівняння» з готовими варіантами (є лише на сторінці запису).
  let {
    latency,
    frameGen = false,
    onCompareReflex,
  }: { latency: LatencySummary; frameGen?: boolean; onCompareReflex?: () => void } = $props();

  const ms = (v: number | null) => (v != null ? v.toFixed(1) : "—");
  const parts = $derived([
    { label: tk("ui.lat.cpu_busy"), value: latency.cpu_busy_ms },
    { label: tk("ui.lat.gpu_start"), value: latency.gpu_start_ms },
    { label: tk("ui.lat.until_displayed"), value: latency.until_displayed_ms },
  ]);
  const refs = $derived([
    { label: tk("ui.lat.input_to_photon"), stat: latency.input_to_photon },
    { label: tk("ui.lat.click_to_photon"), stat: latency.click_to_photon },
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
    {@render tile(tk("ui.lat.pc_latency"), latency.pc_latency, tk("ui.lat.none_pcl"))}
    {@render tile(tk("ui.lat.frame_to_display"), latency.frame_to_display, tk("ui.lat.none_frames"))}
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
  <div class="reference">
    <span class="small">{tk("ui.lat.reference")}</span>
    <dl class="parts">
      {#each refs as r (r.label)}
        <div>
          <dt class="small muted">{r.label}</dt>
          <dd class="mono">{r.stat ? tk("ui.lat.ref_value", { v: ms(r.stat.median_ms), n: r.stat.samples }) : "—"}</dd>
        </div>
      {/each}
    </dl>
    <p class="small muted">{tk("ui.lat.reference_note")}</p>
  </div>
  {#if frameGen}<p class="small warn">{tk("ui.lat.frame_gen")}</p>{/if}
  {#if onCompareReflex && latency.pc_latency && !frameGen}
    <div class="compare">
      <Button small variant="secondary" onclick={onCompareReflex}>{tk("ui.lat.compare_reflex")}</Button>
      <span class="small muted">{tk("ui.lat.compare_reflex_hint")}</span>
    </div>
  {/if}
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
  .reference {
    display: grid;
    gap: calc(var(--space) * 2);
    padding-top: calc(var(--space) * 3);
    border-top: 1px solid var(--border);
  }
  .compare {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
  .warn {
    color: var(--warn);
  }
  p {
    margin: 0;
  }
</style>
