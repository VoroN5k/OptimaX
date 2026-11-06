<script lang="ts">
  import type { ExpEstimate } from "../lib/bindings/ExpEstimate";
  import type { ExpReport } from "../lib/bindings/ExpReport";
  import { tk } from "../lib/i18n";

  // Інтервали ефекту кожної зміни відносно базового варіанта (A) — з рушія порівнянь.
  // Спільне для «Порівняння», A/B-тесту оптимізації й тесту роздільності.
  let { report, names }: { report: ExpReport; names: string[] } = $props();

  const pct = (v: number) => `${v >= 0 ? "+" : ""}${v.toFixed(1)}`;
  const letter = (v: number) => String.fromCharCode(65 + v);
  const nameOf = (v: number) => names[v] ?? letter(v);
  const shown = $derived(report.comparisons.filter((c) => c.avg_fps && c.low_1pct));
  const drift = $derived(shown[0]?.avg_fps?.drift_pct_per_run ?? null);

  /** Шкала інтервалу: нуль, смуга «без ефекту» (±поріг) і інтервал з точкою. */
  function bar(e: ExpEstimate, margin: number) {
    const lo = Math.min(e.ci_low_pct, -margin * 3);
    const hi = Math.max(e.ci_high_pct, margin * 3);
    const at = (v: number) => ((v - lo) / (hi - lo)) * 100;
    return {
      zero: at(0),
      band: [at(-margin), at(margin)] as const,
      ci: [at(e.ci_low_pct), at(e.ci_high_pct)] as const,
      point: at(e.delta_pct),
    };
  }
</script>

{#snippet estimate(label: string, e: ExpEstimate, margin: number)}
  {@const b = bar(e, margin)}
  <div class="est">
    <div class="est-head">
      <span class="small muted">{label}</span>
      <span class="mono">{pct(e.delta_pct)} %</span>
      <span class="small dim mono">{tk("ui.exp.ci", { lo: pct(e.ci_low_pct), hi: pct(e.ci_high_pct) })}</span>
      <span class="out small {e.outcome}">{tk(`ui.exp.outcome.${e.outcome}`)}</span>
    </div>
    <div class="scale" aria-hidden="true">
      <i class="band" style:left="{b.band[0]}%" style:width="{b.band[1] - b.band[0]}%"></i>
      <i class="zero" style:left="{b.zero}%"></i>
      <i class="ci {e.outcome}" style:left="{b.ci[0]}%" style:width="{Math.max(b.ci[1] - b.ci[0], 0.5)}%"></i>
      <i class="pt" style:left="{b.point}%"></i>
    </div>
  </div>
{/snippet}

{#if shown.length}
  <div class="ests">
    {#each shown as c (c.variant)}
      <div class="cmp">
        {#if report.variants > 2}
          <span class="small cmp-title">{tk("ui.exp.vs", { b: nameOf(c.variant), a: nameOf(0) })}</span>
        {/if}
        {#if c.avg_fps}{@render estimate(tk("ui.exp.avg"), c.avg_fps, report.margin_avg_pct)}{/if}
        {#if c.low_1pct}{@render estimate(tk("ui.exp.low"), c.low_1pct, report.margin_low_pct)}{/if}
        {#if c.pc_latency && report.margin_latency_pct != null}
          {@render estimate(
            tk("ui.exp.latency", { a: c.pc_latency.a.toFixed(1), b: c.pc_latency.b.toFixed(1) }),
            c.pc_latency,
            report.margin_latency_pct,
          )}
        {/if}
      </div>
    {/each}
  </div>
  {#if shown.some((c) => c.pc_latency)}<p class="small dim">{tk("ui.exp.latency_note")}</p>{/if}
  <p class="small dim">
    {tk(report.variants > 2 ? "ui.exp.method_many" : "ui.exp.method", {
      conf: report.confidence_pct.toFixed(1),
      drift: drift != null ? pct(drift) : "—",
    })}
  </p>
{/if}

<style>
  .ests {
    display: grid;
    gap: calc(var(--space) * 4);
    margin-bottom: calc(var(--space) * 2);
  }
  .cmp {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .cmp-title {
    font-weight: 600;
  }
  .est {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .est-head {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: calc(var(--space) * 3);
  }
  .out {
    margin-left: auto;
    color: var(--text-muted);
  }
  .out.better {
    color: var(--ok);
  }
  .out.worse {
    color: var(--danger);
  }
  .scale {
    position: relative;
    height: 14px;
    border-radius: var(--radius-sm);
    background: var(--bg-input);
  }
  .scale i {
    position: absolute;
    top: 0;
    height: 100%;
  }
  .band {
    background: var(--border);
  }
  .zero {
    width: 1px;
    background: var(--text-dim);
  }
  .ci {
    top: 4px !important;
    height: 6px !important;
    border-radius: 3px;
    background: var(--text-muted);
  }
  .ci.better {
    background: var(--ok);
  }
  .ci.worse {
    background: var(--danger);
  }
  .ci.no_effect {
    background: var(--accent-from);
  }
  .pt {
    width: 2px;
    margin-left: -1px;
    background: var(--text);
  }
  p {
    margin: 0;
  }
</style>
