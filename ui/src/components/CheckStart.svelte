<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { CheckResult } from "../lib/bindings/CheckResult";
  import { fps } from "../lib/historyFormat";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";

  // «Перевірити мою гру» на «Огляді»: запускає сценарій у «Діагностиці» (там кроки й запис) і
  // показує підсумок останньої перевірки «було / стало» (висновок рахує бекенд).
  let last = $state<CheckResult | null>(null);

  onMount(() => {
    void api
      .checkLast()
      .then((c) => (last = c))
      .catch(() => (last = null));
  });

  function begin() {
    ui.check = { armed: false, exe: "", seconds: 120, scene: "", baseline: null, outcome: null, rechecking: false, afterId: null, choice: null, mainAdvice: null };
    ui.screen = "diagnosis";
  }

  const summary = $derived.by(() => {
    if (!last) return null;
    const c = last.comparison;
    const lows = last.focus === "lows";
    return tk("ui.check.last", {
      game: c.newer.game,
      metric: tk(`ui.check.metric.${last.focus}`),
      from: fps(lows ? c.older.low_1pct_fps : c.older.avg_fps),
      to: fps(lows ? c.newer.low_1pct_fps : c.newer.avg_fps),
      verdict: tk(`ui.check.verdict.${last.trend}`),
    });
  });
</script>

<Card title={tk("ui.check.title")}>
  <div class="row">
    <p class="text">{tk("ui.check.text")}</p>
    <Button variant="primary" icon="play" onclick={begin}>{tk("ui.check.start")}</Button>
  </div>
  {#if summary && last}
    <p class="last small muted {last.trend}"><i class="dot" aria-hidden="true"></i>{summary}</p>
  {/if}
</Card>

<style>
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 4);
  }
  .text {
    flex: 1;
    min-width: 240px;
    margin: 0;
    color: var(--text-2);
  }
  .last {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    margin: calc(var(--space) * 3) 0 0;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
  }
  .better .dot {
    background: var(--ok);
  }
  .worse .dot {
    background: var(--warn);
  }
</style>
