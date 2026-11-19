<script lang="ts">
  import type { CheckResult } from "../lib/bindings/CheckResult";
  import { fps, pct } from "../lib/historyFormat";
  import { t, tk } from "../lib/i18n";
  import Button from "./Button.svelte";
  import ComparisonSummary from "./ComparisonSummary.svelte";

  // «Було / стало» сценарію «Перевірити мою гру»: висновок за метрикою дії (рахує бекенд,
  // `history::check`), під ним — звичне порівняння записів (умови, що змінилося в ПК).
  let {
    res,
    onCompare,
  }: { res: CheckResult; onCompare?: (game: string, variants: string[]) => void } = $props();

  const cmp = $derived(res.comparison);
  const from = $derived(res.focus === "lows" ? cmp.older.low_1pct_fps : cmp.older.avg_fps);
  const to = $derived(res.focus === "lows" ? cmp.newer.low_1pct_fps : cmp.newer.avg_fps);
  const line = $derived(
    tk(`ui.check.trend.${res.trend}.${res.focus}`, {
      from: fps(from),
      to: fps(to),
      pct: pct(res.change_pct),
      t: cmp.threshold_pct.toFixed(0),
    }),
  );
</script>

<section class="tile check" aria-label={tk("ui.check.card")}>
  <div class="micro">{tk("ui.check.card")}</div>
  <p class="verdict {res.trend}" role="status">
    <i class="dot" aria-hidden="true"></i>
    <span>
      {line}
      {#if res.trend === "better"}<strong>{tk("ui.check.keep")}</strong>{/if}
      {#if res.trend === "worse"}<strong>{tk("ui.check.revert")}</strong>{/if}
    </span>
  </p>
  {#if res.ab && onCompare}
    {@const ab = res.ab}
    <div class="ab">
      <Button small variant="secondary" onclick={() => onCompare(cmp.newer.game, ab.map((v) => t(v)))}>{tk("ui.check.ab")}</Button>
      <span class="small muted">{tk("ui.check.ab_note")}</span>
    </div>
  {/if}
  {#if res.advice}
    <p class="changed small"><span class="muted">{tk("ui.check.changed")}</span> {t(res.advice)}</p>
  {/if}
  <ComparisonSummary {cmp} compact trend={false} />
</section>

<style>
  .check {
    display: grid;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 5);
  }
  .verdict {
    display: flex;
    align-items: baseline;
    gap: calc(var(--space) * 2);
    margin: 0;
    font-size: 15px;
    color: var(--text);
  }
  .verdict strong {
    font-weight: 600;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
  }
  .verdict.better .dot {
    background: var(--ok);
  }
  .verdict.worse .dot {
    background: var(--warn);
  }
  .changed {
    margin: 0;
  }
  .ab {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
</style>
