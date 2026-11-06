<script lang="ts">
  import type { Snippet } from "svelte";
  import type { DiagnosisResult } from "../lib/bindings/DiagnosisResult";
  import { t, tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import FindingCard from "./FindingCard.svelte";
  import Fold from "./Fold.svelte";
  import FrameAnatomy from "./FrameAnatomy.svelte";
  import LatencyCard from "./LatencyCard.svelte";
  import FrametimeChart from "./FrametimeChart.svelte";
  import TimelineExplorer from "./TimelineExplorer.svelte";

  // Результат запису: підсумок, знахідки, радар. Спільний для «Чому лагає?» та «Історії».
  let {
    result,
    example = false,
    hideResFindings = false,
    after,
  }: {
    result: DiagnosisResult;
    /** Позначений приклад знайомства (не виміряні дані). */
    example?: boolean;
    /** Підсумок тесту роздільності вже показано окремо — у списку не дублювати. */
    hideResFindings?: boolean;
    /** Що показати після радара (напр. тест роздільності). */
    after?: Snippet;
  } = $props();

  let chartOpen = $state(false);

  const stats = $derived(result.diagnosis.stats);
  const summary = $derived(result.diagnosis.verdicts[0]?.finding ?? null);
  /** Цифри, що вже стоять плитками вище, — у списку доказів підсумку не повторюємо. */
  const TILE_KEYS = new Set(["diag.ev.avg_fps", "diag.ev.low1", "diag.ev.low01", "diag.ev.p99"]);
  const extra = $derived((summary?.evidence ?? []).filter((e) => !TILE_KEYS.has(e.label.key)));
  const rest = $derived(
    result.diagnosis.verdicts.slice(1).filter((v) => !(hideResFindings && v.finding.id.startsWith("res."))),
  );
</script>

{#snippet openUpscale()}
  <Button small variant="secondary" icon="upscale" onclick={() => (ui.screen = "upscale")}>{tk("ui.up.open")}</Button>
{/snippet}

{#if summary}
  {#if example}
    <div class="example" role="note">
      <strong>{tk("ui.tour.example_title")}</strong>
      <span>{tk("ui.tour.example_text")}</span>
    </div>
  {/if}
  <div data-tour="diag-summary">
    <Card title={t(summary.title)} subtitle={t(summary.detail)}>
      {#if stats}
        <div class="stats" data-tour="diag-stats">
          <div><strong>{stats.avg_fps.toFixed(1)}</strong><span>{tk("ui.diag.avg")}</span></div>
          <div><strong>{stats.low_1pct_fps.toFixed(1)}</strong><span>1% low</span></div>
          <div><strong>{stats.low_01pct_fps.toFixed(1)}</strong><span>0.1% low</span></div>
          <div><strong>{stats.p99_frametime_ms.toFixed(2)}</strong><span>{tk("ui.diag.p99")}</span></div>
        </div>
      {/if}
      {#if extra.length}
        <p class="small muted load-title">{tk("ui.diag.load")}</p>
        <dl class="load">
          {#each extra as e, i (i)}
            <dt>{t(e.label)}</dt>
            <dd class="mono">{t(e.value)}</dd>
          {/each}
        </dl>
      {/if}
      <p class="small dim">
        {tk("ui.diag.confidence", { v: summary.confidence })}
        {#if result.display_hz} · {tk("ui.diag.hz", { hz: result.display_hz })}{/if}
        · {tk("ui.rec.counted_total", { s: Math.round(result.counted_s) })}
        {#if result.end_reason !== "completed"} · {tk(`ui.rec.end.${result.end_reason}`)}{/if}
      </p>
    </Card>
  </div>

  <section class="findings" data-tour="diag-findings" aria-label={tk("ui.diag.findings")}>
    <h2 class="section muted">{tk("ui.diag.findings_n", { n: rest.length })}</h2>
    {#if rest.length}
      <div class="list">
        {#each rest as v, i (`${result.diagnosis.game}-${i}-${v.finding.id}`)}
          <FindingCard
            finding={v.finding}
            collapsible
            expanded={i === 0}
            action={v.finding.id === "upscale.helps" ? openUpscale : undefined}
          />
        {/each}
      </div>
    {:else}
      <p class="none small muted">{tk("ui.diag.no_more")}</p>
    {/if}
  </section>

  {#if result.diagnosis.chart}
    <div data-tour="diag-radar">
      <Fold
        title={tk("ui.chart.title")}
        summary={tk("ui.chart.spikes", { n: result.diagnosis.chart.spikes_total })}
        bind:open={chartOpen}
      >
        <p class="small muted fold-hint">{tk("ui.chart.subtitle")}</p>
        <FrametimeChart chart={result.diagnosis.chart} />
      </Fold>
    </div>
  {/if}
  {#if result.diagnosis.anatomy}
    {@const an = result.diagnosis.anatomy}
    <Fold
      title={tk("ui.anatomy.title")}
      summary={tk("ui.anatomy.summary", { ms: an.typical.frame_ms.toFixed(1), n: an.spikes.length })}
    >
      <FrameAnatomy anatomy={an} start={result.diagnosis.view?.start_ms ?? result.diagnosis.chart?.start_ms ?? 0} />
    </Fold>
  {/if}
  {#if result.diagnosis.latency}
    {@const lat = result.diagnosis.latency}
    <Fold
      title={tk("ui.lat.title")}
      summary={lat.pc_latency
        ? tk("ui.lat.summary_pcl", { ms: lat.pc_latency.median_ms.toFixed(1) })
        : lat.frame_to_display
          ? tk("ui.lat.summary", { ms: lat.frame_to_display.median_ms.toFixed(1) })
          : tk("ui.lat.summary_input")}
    >
      <LatencyCard latency={lat} frameGen={result.diagnosis.frame_gen != null} />
    </Fold>
  {/if}
  {#if result.diagnosis.view && result.diagnosis.view.frames.t.length}
    {@const v = result.diagnosis.view}
    <Fold
      title={tk("ui.tl.title")}
      summary={tk("ui.tl.summary", { s: ((v.end_ms - v.start_ms) / 1000).toFixed(0), n: v.spikes.length })}
    >
      <p class="small muted fold-hint">{tk("ui.tl.subtitle")}</p>
      <TimelineExplorer view={v} chart={result.diagnosis.chart} />
    </Fold>
  {/if}
  {@render after?.()}
  {#if result.presentmon_messages.length}
    <details class="pm small muted">
      <summary>{tk("ui.diag.pm_messages")}</summary>
      <pre class="mono">{result.presentmon_messages.join("\n")}</pre>
    </details>
  {/if}
{/if}

<style>
  .example {
    display: grid;
    gap: 2px;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px dashed var(--warn);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    font-size: 13px;
  }
  .example strong {
    color: var(--warn);
  }
  .example span {
    color: var(--text-muted);
  }
  .findings {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .section {
    margin: calc(var(--space) * 1) 0 0;
    padding: 0 calc(var(--space) * 1);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: 11px;
  }
  .fold-hint {
    margin: 0 0 calc(var(--space) * 3);
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: calc(var(--space) * 3);
    margin-bottom: calc(var(--space) * 3);
  }
  .stats div {
    display: grid;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    border: 1px solid var(--border);
  }
  .stats strong {
    font-size: 24px;
    font-weight: 600;
  }
  .stats span {
    font-size: 12px;
    color: var(--text-muted);
  }
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .load-title {
    margin: 0 0 calc(var(--space) * 2);
  }
  .load {
    display: grid;
    grid-template-columns: minmax(160px, max-content) 1fr;
    gap: 4px calc(var(--space) * 4);
    margin: 0 0 calc(var(--space) * 3);
    padding: calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-size: 12px;
  }
  .load dt {
    color: var(--text-muted);
  }
  .load dd {
    margin: 0;
  }
  .none {
    margin: 0;
    padding: 0 calc(var(--space) * 1);
  }
  .pm pre {
    white-space: pre-wrap;
  }
</style>
