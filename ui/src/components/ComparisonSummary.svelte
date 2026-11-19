<script lang="ts">
  import type { RecordComparison } from "../lib/bindings/RecordComparison";
  import { fps, pct, stateValue } from "../lib/historyFormat";
  import { t, tk } from "../lib/i18n";

  // Порівняння двох записів (старіший → новіший): дельти, висновок з порогом похибки, попередження
  // про різні умови й що змінилося в ПК. Усе рахує бекенд (`history::compare`). Спільне для
  // «Історії» й картки «Проти попереднього запису» в результаті; `compact` — до трьох змін.
  // `trend = false` — висновок за середнім FPS не показуємо (у «Було / стало» свій висновок за
  // метрикою дії, щоб два рядки не суперечили).
  let { cmp, compact = false, trend = true }: { cmp: RecordComparison; compact?: boolean; trend?: boolean } = $props();

  const changes = $derived(compact ? cmp.changes.slice(0, 3) : cmp.changes);
</script>

<div class="cmp" class:compact>
  <div class="delta {cmp.trend}">
    <span class="small muted">{tk("ui.diag.avg")}</span>
    <strong class="mono">{fps(cmp.older.avg_fps)} → {fps(cmp.newer.avg_fps)}</strong>
    <span class="mono">{pct(cmp.avg_fps_change_pct)}</span>
  </div>
  <div class="delta">
    <span class="small muted">1% low</span>
    <strong class="mono">{fps(cmp.older.low_1pct_fps)} → {fps(cmp.newer.low_1pct_fps)}</strong>
    <span class="mono">{pct(cmp.low_1pct_change_pct)}</span>
  </div>
  {#if trend}
    <p class="trend {cmp.trend}" role="status">
      <i class="dot" aria-hidden="true"></i>
      {tk(`ui.history.trend.${cmp.trend}`, { t: cmp.threshold_pct.toFixed(0) })}
    </p>
  {/if}
</div>
{#if !cmp.same_game}
  <p class="warn small">{tk("ui.history.warn_game")}</p>
{/if}
{#if cmp.scene === "different"}
  <p class="warn small">{tk("ui.history.warn_scene", { a: cmp.older.scene ?? "", b: cmp.newer.scene ?? "" })}</p>
{:else if cmp.scene === "unknown"}
  <p class="small muted note">{tk("ui.history.scene_unknown")}</p>
{/if}
{#if !cmp.same_display}
  <p class="warn small">{tk("ui.history.warn_display")}</p>
{/if}
{#if cmp.bottleneck_changed}
  <p class="small">{tk("ui.history.bottleneck_changed", { from: t(cmp.older.title), to: t(cmp.newer.title) })}</p>
{/if}
<h3 class="section muted">{tk("ui.history.what_changed")}</h3>
{#if changes.length}
  <dl class="changes" class:stacked={compact}>
    {#each changes as ch (ch.field)}
      <dt>{tk(`ui.history.field.${ch.field}`)}</dt>
      <dd class="mono">{stateValue(ch, ch.from)} → {stateValue(ch, ch.to)}</dd>
    {/each}
  </dl>
{:else}
  <p class="small muted">{tk("ui.history.no_changes")}</p>
{/if}

<style>
  .note {
    margin: calc(var(--space) * 3) 0 0;
  }
  .cmp {
    display: flex;
    flex-wrap: wrap;
    align-items: stretch;
    gap: calc(var(--space) * 3);
  }
  .delta {
    display: grid;
    gap: 2px;
    min-width: 180px;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  .compact .delta {
    flex: 1;
    min-width: 120px;
  }
  .delta strong {
    font-size: 20px;
    font-weight: 600;
  }
  .trend {
    flex: 1;
    min-width: 220px;
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    margin: 0;
    font-size: 14px;
  }
  .compact .trend {
    flex-basis: 100%;
    min-width: 0;
    font-size: 13px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
  }
  .trend.better .dot {
    background: var(--ok);
  }
  .trend.worse .dot {
    background: var(--warn);
  }
  .warn {
    margin: calc(var(--space) * 3) 0 0;
    color: var(--warn);
  }
  .section {
    margin: calc(var(--space) * 4) 0 calc(var(--space) * 2);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .changes {
    display: grid;
    grid-template-columns: minmax(160px, max-content) 1fr;
    gap: 4px calc(var(--space) * 4);
    margin: 0;
    padding: calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-size: 13px;
  }
  .changes dt {
    color: var(--text-muted);
  }
  .changes dd {
    margin: 0;
    word-break: break-word;
  }
  /* У вузькій картці — підпис над значенням: «3733 MT/s → 3600 MT/s» не ламається посередині. */
  .changes.stacked {
    grid-template-columns: 1fr;
    gap: 0;
  }
  .changes.stacked dd + dt {
    margin-top: calc(var(--space) * 2);
  }
</style>
