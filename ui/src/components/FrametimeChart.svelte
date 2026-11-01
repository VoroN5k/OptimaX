<script lang="ts">
  import type { FrametimeChart } from "../lib/bindings/FrametimeChart";
  import { t, tk } from "../lib/i18n";

  // Радар мікрофризів: графік frametime (найдовший кадр у кожному відрізку часу) і позначки
  // ривків. Причини ривків визначає лише бекенд; тут — тільки показ.
  let { chart }: { chart: FrametimeChart } = $props();

  /** Внутрішні координати SVG (розтягується на ширину картки). */
  const W = 1000;
  const H = 220;

  // Шкала — від першого зарахованого кадру (очікування гри й прогрів не показуємо).
  const start = $derived(chart.start_ms);
  const span = $derived(Math.max(chart.duration_ms - chart.start_ms, 1));
  const maxAll = $derived(Math.max(...chart.points.map((p) => p.max_ms), chart.median_ms, 1));
  // Межа шкали: один ривок на 300 мс не має сплющувати весь графік у лінію.
  const yMax = $derived(Math.max(chart.median_ms * 3, Math.min(maxAll, Math.max(chart.median_ms * 12, 50))));

  const x = (ms: number) => ((ms - start) / span) * W;
  const y = (ms: number) => H - (Math.min(ms, yMax) / yMax) * H;

  /** Ширина одного відрізка графіка, мс. */
  const bucket = $derived(span / Math.max(chart.points.length, 1));
  /** Розрив, довший за це, — не зараховано (Alt+Tab, меню): лінію не з'єднуємо. */
  const gapMs = $derived(Math.max(bucket * 4, 250));

  type Point = (typeof chart.points)[number];
  /** Безперервна ділянка графіка між розривами. */
  type Run = { points: Point[]; first: Point; last: Point };

  const runs = $derived.by(() => {
    const out: Run[] = [];
    let cur: Run | null = null;
    for (const p of chart.points) {
      if (cur && p.t_ms - cur.last.t_ms <= gapMs) {
        cur.points.push(p);
        cur.last = p;
      } else {
        cur = { points: [p], first: p, last: p };
        out.push(cur);
      }
    }
    return out;
  });
  /** Розриви між ділянками: [від, до], мс. */
  const gaps = $derived.by(() => {
    const out: [number, number][] = [];
    let prev: Run | null = null;
    for (const r of runs) {
      if (prev) out.push([prev.last.t_ms + bucket, r.first.t_ms]);
      prev = r;
    }
    return out;
  });

  const pt = (t: number, ms: number) => `${x(t).toFixed(1)},${y(ms).toFixed(1)}`;
  const maxPath = $derived(
    runs
      .map((r) => {
        const end = x(r.last.t_ms + bucket).toFixed(1);
        const line = r.points.map((p) => `L${pt(p.t_ms, p.max_ms)}`).join(" ");
        return `M${x(r.first.t_ms).toFixed(1)},${H} ${line} L${end},${y(r.last.max_ms).toFixed(1)} L${end},${H} Z`;
      })
      .join(" "),
  );
  const avgPath = $derived(
    runs.map((r) => r.points.map((p, i) => `${i ? "L" : "M"}${pt(p.t_ms, p.avg_ms)}`).join(" ")).join(" "),
  );

  /** Обраний ривок: за замовчуванням — найдовший. */
  let selected = $state(0);
  $effect(() => {
    let worst = 0;
    let longest = -1;
    chart.spikes.forEach((s, i) => {
      if (s.frametime_ms > longest) {
        longest = s.frametime_ms;
        worst = i;
      }
    });
    selected = worst;
  });
  const current = $derived(chart.spikes[selected]);

  let markers = $state<HTMLButtonElement[]>([]);
  function move(to: number) {
    if (!chart.spikes.length) return;
    selected = Math.max(0, Math.min(chart.spikes.length - 1, to));
    markers[selected]?.focus();
  }
  function onKey(e: KeyboardEvent) {
    const next = { ArrowRight: selected + 1, ArrowLeft: selected - 1, Home: 0, End: chart.spikes.length - 1 }[e.key];
    if (next !== undefined) {
      e.preventDefault();
      move(next);
    }
  }

  /** Найдовший ривок, що відповідає умові (для переходу зі зведення). */
  function showWorst(pred: (s: (typeof chart.spikes)[number]) => boolean) {
    let best = -1;
    let longest = -1;
    chart.spikes.forEach((s, i) => {
      if (pred(s) && s.frametime_ms > longest) {
        longest = s.frametime_ms;
        best = i;
      }
    });
    if (best >= 0) move(best);
  }

  /** Секунди від початку шкали. */
  const sec = (ms: number) => (Math.max(0, ms - start) / 1000).toFixed(1);
</script>

<div class="radar">
  <div class="head">
    <span class="small muted">
      {tk("ui.chart.spikes", { n: chart.spikes_total })}
      {#if chart.spikes.length < chart.spikes_total} · {tk("ui.chart.shown", { n: chart.spikes.length })}{/if}
    </span>
    <span class="legend small muted">
      <span><i class="dash" aria-hidden="true"></i>{tk("ui.chart.normal", { v: chart.median_ms.toFixed(1) })}</span>
      <span aria-hidden="true"><i class="dot found"></i>{tk("ui.chart.legend_found")}</span>
      <span aria-hidden="true"><i class="dot unknown"></i>{tk("ui.chart.legend_unknown")}</span>
    </span>
  </div>

  <div class="plot">
    <span class="ylab top mono small dim">{yMax.toFixed(0)} {tk("ui.chart.ms")}</span>
    <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label={tk("ui.chart.aria", { n: chart.spikes_total })}>
      <defs>
        <linearGradient id="ft-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stop-color="var(--accent-to)" stop-opacity="0.35" />
          <stop offset="100%" stop-color="var(--accent-from)" stop-opacity="0.04" />
        </linearGradient>
      </defs>
      {#each gaps as [from, to] (from)}
        <rect x={x(from)} width={Math.max(x(to) - x(from), 1)} y="0" height={H} class="gap" />
      {/each}
      <path d={maxPath} fill="url(#ft-fill)" stroke="var(--accent-to)" stroke-width="1" vector-effect="non-scaling-stroke" />
      <path d={avgPath} fill="none" stroke="var(--accent-from)" stroke-width="1.5" vector-effect="non-scaling-stroke" />
      <line x1="0" x2={W} y1={y(chart.median_ms)} y2={y(chart.median_ms)} class="median" vector-effect="non-scaling-stroke" />
    </svg>
    {#each gaps as [from, to] (from)}
      <span class="gaplab small dim" style="left: {(x((from + to) / 2) / W) * 100}%" title={tk("ui.chart.gap_hint")}>
        {tk("ui.chart.gap")}
      </span>
    {/each}

    <div class="marks" role="listbox" tabindex="-1" aria-label={tk("ui.chart.list")} onkeydown={onKey}>
      {#each chart.spikes as s, i (s.t_ms)}
        <button
          bind:this={markers[i]}
          type="button"
          role="option"
          aria-selected={i === selected}
          tabindex={i === selected ? 0 : -1}
          class="mark"
          class:found={s.causes.length > 0}
          class:sel={i === selected}
          style="left: {(x(s.t_ms) / W) * 100}%; top: {(y(s.frametime_ms) / H) * 100}%"
          aria-label={tk("ui.chart.mark", { t: sec(s.t_ms), v: s.frametime_ms.toFixed(1) })}
          onclick={() => (selected = i)}
        ></button>
      {/each}
    </div>
  </div>
  <div class="xaxis mono small dim" aria-hidden="true">
    <span>0 {tk("ui.chart.s")}</span>
    <span>{sec(start + span / 2)} {tk("ui.chart.s")}</span>
    <span>{sec(start + span)} {tk("ui.chart.s")}</span>
  </div>

  {#if chart.spikes.length && (chart.summary.length || chart.unexplained)}
    <div class="summary">
      <span class="small muted">{tk("ui.chart.summary")}:</span>
      {#each chart.summary as c (c.code)}
        {@const usual = (c.usual * 100).toFixed(0)}
        <button
          type="button"
          class="chip"
          class:found={c.above_usual}
          title={tk(c.above_usual ? "ui.chart.above_usual" : "ui.chart.as_usual", { usual })}
          onclick={() => showWorst((s) => s.codes.includes(c.code))}
        >
          {t(c.name)} <span class="mono">{tk("ui.chart.count", { n: c.spikes, total: chart.spikes.length })}</span>
          {#if !c.above_usual}<span class="bg">· {tk("ui.chart.background")}</span>{/if}
        </button>
      {/each}
      {#if chart.unexplained}
        <button type="button" class="chip none" onclick={() => showWorst((s) => s.causes.length === 0)}>
          {tk("ui.chart.unexplained")} <span class="mono">{tk("ui.chart.count", { n: chart.unexplained, total: chart.spikes.length })}</span>
        </button>
      {/if}
    </div>
  {/if}

  {#if current}
    <div class="detail" aria-live="polite">
      <p>
        <strong>{tk("ui.chart.at", { t: sec(current.t_ms) })}</strong>
        <span class="muted">{tk("ui.chart.frame", { v: current.frametime_ms.toFixed(1), base: current.baseline_ms.toFixed(1) })}</span>
      </p>
      {#if current.causes.length}
        <ul>
          {#each current.causes as c, i (i)}
            <li>{t(c)}</li>
          {/each}
        </ul>
      {:else}
        <p class="small muted">{tk("ui.chart.no_cause")}</p>
      {/if}
      <p class="small dim">{tk("ui.chart.hint")}</p>
    </div>
  {:else}
    <p class="small muted">{tk("ui.chart.smooth")}</p>
  {/if}
</div>

<style>
  .radar {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .head {
    display: flex;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
    flex-wrap: wrap;
  }
  .summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: calc(var(--space) * 1.5);
    padding: calc(var(--space) * 1) calc(var(--space) * 2.5);
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--bg-input);
    color: var(--text-muted);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: border-color var(--dur-fast) var(--ease);
  }
  .chip::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .chip.found {
    color: var(--text);
  }
  .chip.found::before {
    background: var(--warn);
  }
  .chip.none::before {
    background: var(--danger);
  }
  .chip .bg {
    color: var(--text-dim);
  }
  .chip:hover {
    border-color: var(--border-strong);
  }
  .chip:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
  }
  .chip .mono {
    color: var(--text-dim);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 4);
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: calc(var(--space) * 1.5);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
  }
  .dot.found {
    background: var(--warn);
  }
  .dot.unknown {
    background: var(--danger);
  }
  .dash {
    width: 14px;
    border-top: 1px dashed var(--text-dim);
    display: inline-block;
  }
  .plot {
    position: relative;
    height: 220px;
    border-radius: var(--radius-md);
    background: var(--bg-input);
    border: 1px solid var(--border);
  }
  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
  }
  .gap {
    fill: var(--bg-app);
    opacity: 0.6;
  }
  .gaplab {
    position: absolute;
    bottom: calc(var(--space) * 2);
    transform: translateX(-50%);
    white-space: nowrap;
    pointer-events: auto;
  }
  .median {
    stroke: var(--text-dim);
    stroke-width: 1;
    stroke-dasharray: 4 4;
  }
  .ylab {
    position: absolute;
    right: calc(var(--space) * 2);
    transform: translateY(-120%);
    padding: 0 calc(var(--space) * 1.5);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    pointer-events: none;
    z-index: 1;
  }
  .ylab.top {
    top: calc(var(--space) * 5);
  }
  .marks {
    position: absolute;
    inset: 0;
  }
  .mark {
    position: absolute;
    width: 12px;
    height: 12px;
    margin: -6px 0 0 -6px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid var(--bg-card);
    background: var(--danger);
    cursor: pointer;
    transition: transform var(--dur-fast) var(--ease);
  }
  .mark.found {
    background: var(--warn);
  }
  .mark:hover {
    transform: scale(1.3);
  }
  .mark.sel {
    transform: scale(1.45);
    box-shadow: 0 0 0 2px var(--text);
  }
  .mark:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
  }
  .xaxis {
    display: flex;
    justify-content: space-between;
  }
  .detail {
    display: grid;
    gap: calc(var(--space) * 1.5);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-input);
  }
  .detail p {
    margin: 0;
    display: flex;
    gap: calc(var(--space) * 3);
    flex-wrap: wrap;
  }
  .detail ul {
    margin: 0;
    padding-left: calc(var(--space) * 5);
  }
</style>
