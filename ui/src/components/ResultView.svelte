<script lang="ts">
  import type { Snippet } from "svelte";
  import type { DiagnosisResult } from "../lib/bindings/DiagnosisResult";
  import type { Finding } from "../lib/bindings/Finding";
  import { findingRing, splitTitle } from "../lib/findingRing";
  import { t, tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import { api } from "../lib/api";
  import type { RecordComparison } from "../lib/bindings/RecordComparison";
  import type { CheckChoice } from "../lib/bindings/CheckChoice";
  import type { CheckResult } from "../lib/bindings/CheckResult";
  import CheckCard from "./CheckCard.svelte";
  import CheckRow from "./CheckRow.svelte";
  import ComparisonSummary from "./ComparisonSummary.svelte";
  import Disclosure from "./Disclosure.svelte";
  import DualRing from "./DualRing.svelte";
  import FindingCard from "./FindingCard.svelte";
  import Fold from "./Fold.svelte";
  import FrameAnatomy from "./FrameAnatomy.svelte";
  import FrametimeChart from "./FrametimeChart.svelte";
  import HeroCard from "./HeroCard.svelte";
  import ImpactBadge from "./ImpactBadge.svelte";
  import KpiStrip from "./KpiStrip.svelte";
  import LatencyCard from "./LatencyCard.svelte";
  import StoryCard from "./StoryCard.svelte";
  import TimelineExplorer from "./TimelineExplorer.svelte";

  // Результат запису «Pro Premium» (docs/DESIGN.md розд. 6): смуга показників, головна знахідка
  // з кільцем (числа — лише з доказів) і «Що робити далі» з кнопками наступного кроку (крок вирішує
  // бекенд, `Finding.next`), решта знахідок без повтору порад, розбори — в одній згортці «Для
  // досвідчених». Спільний для «Чому лагає?» та «Історії».
  let {
    result,
    example = false,
    hideResFindings = false,
    after,
    onCompare,
    historyId = null,
    live = false,
    next,
    onTry,
    tried = null,
  }: {
    result: DiagnosisResult;
    /** Позначений приклад знайомства (не виміряні дані). */
    example?: boolean;
    /** Підсумок тесту роздільності вже показано окремо — у списку не дублювати. */
    hideResFindings?: boolean;
    /** Що показати після розборів (напр. тест роздільності). */
    after?: Snippet;
    /** Відкрити «Порівняння» з грою й назвами варіантів (немає в прикладі). */
    onCompare?: (game: string, variants: string[]) => void;
    /** Id запису в історії: є — показати порівняння з попереднім записом гри (вибирає бекенд). */
    historyId?: string | null;
    /** Під розповіддю замість підказки «запиши ще раз» (крок сценарію «Перевірити мою гру»). */
    next?: Snippet;
    /** Сценарій: «Спробувати» пораду розповіді та id обраної. */
    onTry?: (f: Finding) => void;
    tried?: CheckChoice | null;
    /** Щойно записаний результат (не з «Історії» й не приклад): лише тоді підказка «запиши ще раз». */
    live?: boolean;
  } = $props();

  let previous = $state<RecordComparison | null>(null);
  /** Запис — перевірка дії: «Було / стало» замість «Проти попереднього запису». */
  let check = $state<CheckResult | null>(null);
  $effect(() => {
    const id = historyId;
    previous = null;
    check = null;
    if (!id) return;
    let alive = true;
    api
      .checkResult(id)
      .then((c) => alive && (check = c))
      .catch((e: unknown) => console.warn("було / стало:", e));
    api
      .historyVsPrevious(id)
      .then((c) => alive && (previous = c))
      .catch((e: unknown) => console.warn("порівняння з попереднім записом:", e));
    return () => {
      alive = false;
    };
  });

  type Review = "chart" | "anatomy" | "latency" | "timeline";
  let open = $state<Record<Review, boolean>>({ chart: false, anatomy: false, latency: false, timeline: false });
  let expertOpen = $state(false);
  let detailsOpen = $state(false);


  const stats = $derived(result.diagnosis.stats);
  /** Розповідь простими словами (бекенд); немає — у старих результатах, тоді звичний вигляд. */
  const story = $derived(result.diagnosis.story ?? null);
  const summary = $derived(result.diagnosis.verdicts[0]?.finding ?? null);
  /** Цифри, що вже стоять у смузі показників, — у доказах головної знахідки не повторюємо. */
  const KPI_KEYS = new Set(["diag.ev.avg_fps", "diag.ev.low1", "diag.ev.low01", "diag.ev.p99"]);
  const evidence = $derived((summary?.evidence ?? []).filter((e) => !KPI_KEYS.has(e.label.key)));
  const rest = $derived(
    result.diagnosis.verdicts.slice(1).filter((v) => !(hideResFindings && v.finding.id.startsWith("res."))),
  );
  /** «Що робити далі»: знахідки з порадою в порядку діагностики (до трьох). З розповіддю дії вже
   *  стоять угорі в ній — тоді в «Детальніше» поради лишаються в картках знахідок, без повтору. */
  const steps = $derived(
    story ? [] : [summary, ...rest.map((v) => v.finding)].filter((f): f is Finding => !!f?.advice).slice(0, 3),
  );
  /** Знахідки, чия порада вже стоїть угорі, — у картках без повтору (копія, дані бекенду не чіпаємо).
   *  За самим об'єктом, не за id: id може повторюватися (кілька фонових процесів — `background`). */
  const shownAdvice = $derived(new Set<Finding>(steps));
  const cardOf = (f: Finding): Finding => (shownAdvice.has(f) ? { ...f, advice: null } : f);
  /** Крок наступної дії лишається в картці, якщо знахідка не потрапила в трійку вгорі. */
  const cardStep = (f: Finding): boolean =>
    !shownAdvice.has(f) && (f.next?.kind === "open" || (f.next?.kind === "compare" && !!onCompare));
  const compareReflex = $derived(
    onCompare
      ? () => onCompare(result.diagnosis.game, [tk("ui.exp.preset.reflex_off"), tk("ui.exp.preset.reflex_on")])
      : undefined,
  );
  const ring = $derived(
    findingRing(
      result.diagnosis.verdicts.map((v) => v.finding),
      result.diagnosis.bottleneck,
    ),
  );
  const title = $derived(summary ? splitTitle(t(summary.title)) : null);
  const ringAria = $derived(
    ring
      ? [ring.outer, ...(ring.inner ? [ring.inner] : [])]
          .map((p) => tk("ui.diag.ring.aria_part", { label: t(p.evidence.label), v: p.value }))
          .join("; ")
      : "",
  );
  const fix1 = (v: number) => v.toFixed(1);

  /** Назви наявних розборів — підсумок згортки «Для досвідчених». */
  const expertSummary = $derived(
    [
      result.diagnosis.view?.frames.t.length ? tk("ui.tl.title") : null,
      result.diagnosis.chart ? tk("ui.chart.title") : null,
      result.diagnosis.anatomy ? tk("ui.anatomy.title") : null,
      result.diagnosis.latency ? tk("ui.lat.title") : null,
    ]
      .filter(Boolean)
      .join(" · "),
  );
</script>

{#snippet stepAction(f: Finding)}
  {#if f.next?.kind === "compare" && onCompare}
    {@const variants = f.next.variants.map((v) => t(v))}
    <Button small variant="secondary" icon="diagnosis" onclick={() => onCompare(result.diagnosis.game, variants)}>
      {tk("ui.diag.next.compare")}
    </Button>
  {:else if f.next?.kind === "open" && f.next.tool === "upscale"}
    <Button small variant="secondary" icon="upscale" onclick={() => (ui.screen = "upscale")}>{tk("ui.up.open")}</Button>
  {/if}
{/snippet}

{#snippet classic()}
  {#if summary}
  <div data-tour={story ? undefined : "diag-stats"}>
    <KpiStrip
      label={tk("ui.diag.kpi.label")}
      items={[
        { label: tk("ui.diag.kpi.avg"), value: stats ? fix1(stats.avg_fps) : null },
        { label: tk("ui.diag.kpi.low1"), value: stats ? fix1(stats.low_1pct_fps) : null },
        { label: tk("ui.diag.kpi.low01"), value: stats ? fix1(stats.low_01pct_fps) : null },
        { label: tk("ui.diag.kpi.p99"), value: stats ? stats.p99_frametime_ms.toFixed(2) : null },
      ]}
    >
      {#snippet footer()}
        <span>{tk("ui.diag.confidence", { v: summary.confidence })}</span>
        {#if result.display_hz}<span>{tk("ui.diag.hz", { hz: result.display_hz })}</span>{/if}
        <span>{tk("ui.rec.counted_total", { s: Math.round(result.counted_s) })}</span>
        {#if result.end_reason !== "completed"}<span>{tk(`ui.rec.end.${result.end_reason}`)}</span>{/if}
      {/snippet}
    </KpiStrip>
  </div>

  <div class="bento">
    <div class="main" data-tour={story ? undefined : "diag-summary"}>
      <HeroCard label={tk("ui.diag.hero")}>
        <div class="status">
          <ImpactBadge impact={summary.impact} pulse />
          <span class="small muted">{tk("ui.diag.confidence", { v: summary.confidence })}</span>
        </div>
        <div class="hero-grid" class:with-ring={!!ring}>
          <div class="hero-text">
            {#if title}
              <h3 class="hero-title" aria-label={t(summary.title)}>
                {#if title.light}{title.light}<br />{/if}<span class="grad">{title.strong}</span>
              </h3>
              {#if title.tail}<p class="tail">{title.tail}</p>{/if}
            {/if}
            <p class="muted">{t(summary.detail)}</p>
          </div>
          {#if ring}
            <DualRing
              outer={{ value: ring.outer.value, label: t(ring.outer.evidence.label) }}
              inner={ring.inner ? { value: ring.inner.value, label: t(ring.inner.evidence.label) } : undefined}
              center={tk(ring.caption)}
              aria={ringAria}
              size={168}
            />
          {/if}
        </div>
        {#if steps.length}
          <div>
            <div class="micro">{tk("ui.diag.check")}</div>
            <ul class="checks">
              {#each steps as f, i (i)}
                <CheckRow n={i + 1}>
                  {#if f.advice}{t(f.advice)}{/if}
                  {#snippet action()}{@render stepAction(f)}{/snippet}
                </CheckRow>
              {/each}
            </ul>
            {#if live && !example && !story}<p class="small muted rerecord">{tk("ui.diag.next.rerecord")}</p>{/if}
          </div>
        {/if}
        {#if evidence.length}
          <Disclosure title={tk("ui.diag.evidence")} count={evidence.length}>
            <dl class="evidence">
              {#each evidence as e, i (i)}
                <dt>{t(e.label)}</dt>
                <dd class="mono">{t(e.value)}</dd>
              {/each}
            </dl>
          </Disclosure>
        {/if}
      </HeroCard>
    </div>

    <div class="side">
      {#if check && !story}
        <CheckCard res={check} {onCompare} />
      {:else if previous && !story}
        <section class="tile prev" aria-label={tk("ui.prev.title")}>
          <div class="micro">{tk("ui.prev.title")}</div>
          <ComparisonSummary cmp={previous} compact />
          <p class="small muted prev-note">{tk("ui.prev.note")}</p>
        </section>
      {/if}
    </div>
  </div>

  <section class="findings" data-tour={story ? undefined : "diag-findings"} aria-label={tk("ui.diag.findings")}>
    <h2 class="micro">{tk("ui.diag.findings_n", { n: rest.length })}</h2>
    {#if rest.length}
      <div class="list">
        {#each rest as v, i (`${result.diagnosis.game}-${i}-${v.finding.id}`)}
          {#if cardStep(v.finding)}
            <FindingCard finding={cardOf(v.finding)} collapsible expanded={i === 0}>
              {#snippet action()}{@render stepAction(v.finding)}{/snippet}
            </FindingCard>
          {:else}
            <FindingCard finding={cardOf(v.finding)} collapsible expanded={i === 0} />
          {/if}
        {/each}
      </div>
    {:else}
      <p class="none small muted">{tk("ui.diag.no_more")}</p>
    {/if}
  </section>
  {/if}
{/snippet}

{#if summary}
  {#if example}
    <div class="example" role="note">
      <strong>{tk("ui.tour.example_title")}</strong>
      <span>{tk("ui.tour.example_text")}</span>
    </div>
  {/if}

  {#if story}
    <StoryCard {story} verdicts={result.diagnosis.verdicts} {onTry} {tried}>
      {#snippet action(f)}{@render stepAction(f)}{/snippet}
      {#snippet meta()}
        <span>{tk("ui.rec.counted_total", { s: Math.round(result.counted_s) })}</span>
        {#if result.end_reason !== "completed"}<span>{tk(`ui.rec.end.${result.end_reason}`)}</span>{/if}
      {/snippet}
    </StoryCard>
    {#if !next && live && !example && story.outcome !== "insufficient"}
      <p class="small muted rerecord">{tk("ui.diag.next.rerecord")}</p>
    {/if}
    {#if check}
      <CheckCard res={check} {onCompare} />
    {:else if previous}
      <section class="tile prev" aria-label={tk("ui.prev.title")}>
        <div class="micro">{tk("ui.prev.title")}</div>
        <ComparisonSummary cmp={previous} compact />
        <p class="small muted prev-note">{tk("ui.prev.note")}</p>
      </section>
    {/if}
    <!-- Сценарій «Перевірити мою гру»: після висновку «було / стало» — наступний крок. -->
    {@render next?.()}
    <div class="anchor" data-tour="diag-findings">
      <Fold title={tk("ui.diag.details")} summary={tk("ui.diag.details_summary")} bind:open={detailsOpen}>
        <div class="details">{@render classic()}</div>
      </Fold>
    </div>
  {:else}
    {@render classic()}
  {/if}

  {#if expertSummary}
    <Fold title={tk("ui.diag.expert")} summary={expertSummary} bind:open={expertOpen}>
      <div class="expert">
      {#if result.diagnosis.view && result.diagnosis.view.frames.t.length}
        {@const v = result.diagnosis.view}
        <div class="anchor">
          <Fold
            title={tk("ui.tl.title")}
            summary={tk("ui.tl.summary", { s: ((v.end_ms - v.start_ms) / 1000).toFixed(0), n: v.spikes.length })}
            bind:open={open.timeline}
          >
            <p class="small muted fold-hint">{tk("ui.tl.subtitle")}</p>
            <TimelineExplorer view={v} chart={result.diagnosis.chart} />
          </Fold>
        </div>
      {/if}
      {#if result.diagnosis.chart}
        <div class="anchor" data-tour="diag-radar">
          <Fold
            title={tk("ui.chart.title")}
            summary={tk("ui.chart.spikes", { n: result.diagnosis.chart.spikes_total })}
            bind:open={open.chart}
          >
            <p class="small muted fold-hint">{tk("ui.chart.subtitle")}</p>
            <FrametimeChart chart={result.diagnosis.chart} />
          </Fold>
        </div>
      {/if}
      {#if result.diagnosis.anatomy}
        {@const an = result.diagnosis.anatomy}
        <div class="anchor">
          <Fold
            title={tk("ui.anatomy.title")}
            summary={tk("ui.anatomy.summary", { ms: an.typical.frame_ms.toFixed(1), n: an.spikes.length })}
            bind:open={open.anatomy}
          >
            <FrameAnatomy anatomy={an} start={result.diagnosis.view?.start_ms ?? result.diagnosis.chart?.start_ms ?? 0} />
          </Fold>
        </div>
      {/if}
      {#if result.diagnosis.latency}
        {@const lat = result.diagnosis.latency}
        <div class="anchor">
          <Fold
            title={tk("ui.lat.title")}
            summary={lat.pc_latency
              ? tk("ui.lat.summary_pcl", { ms: lat.pc_latency.median_ms.toFixed(1) })
              : lat.frame_to_display
                ? tk("ui.lat.summary", { ms: lat.frame_to_display.median_ms.toFixed(1) })
                : tk("ui.lat.summary_input")}
            bind:open={open.latency}
          >
            <LatencyCard latency={lat} frameGen={result.diagnosis.frame_gen != null} onCompareReflex={compareReflex} />
          </Fold>
        </div>
      {/if}
      </div>
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
    background: var(--fill-2);
    font-size: 13px;
  }
  .example strong {
    color: var(--warn);
  }
  .example span {
    color: var(--text-2);
  }
  .bento {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: var(--gap);
  }
  .main {
    grid-column: span 8;
    min-width: 0;
  }
  .side {
    grid-column: span 4;
    display: flex;
    flex-direction: column;
    gap: var(--gap);
    min-width: 0;
  }
  /* Без бічної колонки головна картка займає всю ширину. */
  .bento:not(:has(.side > *)) .main {
    grid-column: 1 / -1;
  }
  @media (max-width: 900px) {
    .main,
    .side {
      grid-column: 1 / -1;
    }
  }
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 2.5) calc(var(--space) * 3.5);
  }
  .hero-grid {
    display: grid;
    gap: calc(var(--space) * 6);
    align-items: start;
  }
  .hero-grid.with-ring {
    grid-template-columns: repeat(auto-fit, minmax(min(300px, 100%), 1fr));
  }
  .hero-text {
    display: flex;
    flex-direction: column;
    gap: calc(var(--space) * 3.5);
  }
  .hero-title {
    font-size: var(--fs-hero);
    line-height: var(--lh-hero);
    font-weight: 300;
    letter-spacing: -0.035em;
  }
  .grad {
    font-weight: 600;
    background: var(--text-gradient);
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .tail {
    font-size: var(--fs-tile);
    line-height: var(--lh-tile);
    color: var(--text);
  }
  .checks {
    margin: calc(var(--space) * 2) 0 0;
    padding: 0;
  }
  .evidence {
    display: grid;
    grid-template-columns: minmax(160px, max-content) 1fr;
    gap: 4px calc(var(--space) * 4);
    margin: 0;
    font-size: var(--fs-small);
  }
  .evidence dt {
    color: var(--text-2);
  }
  .evidence dd {
    margin: 0;
    color: var(--text);
  }
  .prev {
    display: grid;
    gap: calc(var(--space) * 2);
    align-content: start;
    padding: calc(var(--space) * 5);
  }
  .prev-note {
    margin: calc(var(--space) * 2) 0 0;
  }
  .rerecord {
    margin: calc(var(--space) * 3) 0 0;
  }
  .details {
    display: grid;
    gap: var(--gap);
  }
  .expert {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .findings {
    display: grid;
    gap: calc(var(--space) * 2);
    scroll-margin-top: calc(var(--space) * 4);
  }
  .findings h2 {
    margin: calc(var(--space) * 1) 0 0;
    padding: 0 calc(var(--space) * 1);
  }
  .fold-hint {
    margin: 0 0 calc(var(--space) * 3);
  }
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .none {
    margin: 0;
    padding: 0 calc(var(--space) * 1);
  }
  .anchor {
    scroll-margin-top: calc(var(--space) * 4);
  }
  .pm pre {
    white-space: pre-wrap;
  }
</style>
