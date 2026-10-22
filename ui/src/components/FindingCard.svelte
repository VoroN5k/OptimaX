<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Finding } from "../lib/bindings/Finding";
  import { t, tk } from "../lib/i18n";
  import ImpactBadge from "./ImpactBadge.svelte";
  import Icon from "./Icon.svelte";

  let { finding, action }: { finding: Finding; action?: Snippet } = $props();
  let open = $state(false);
</script>

<article class="finding {finding.impact}">
  <span class="bar" aria-hidden="true"></span>
  <div class="main">
    <div class="top">
      <h3>{t(finding.title)}</h3>
      <div class="meta">
        <ImpactBadge impact={finding.impact} />
        <span class="conf small muted" title={tk("ui.finding.confidence_hint")}
          >{tk("ui.finding.confidence", { v: finding.confidence })}</span
        >
      </div>
    </div>
    <p class="detail">{t(finding.detail)}</p>
    {#if finding.advice}
      <p class="advice"><span aria-hidden="true">→</span> {t(finding.advice)}</p>
    {/if}
    <div class="foot">
      {#if finding.evidence.length}
        <button type="button" class="ev-toggle small" aria-expanded={open} onclick={() => (open = !open)}>
          <span class="chev" class:open><Icon name="chevron" size={14} /></span>
          {tk("ui.finding.evidence", { n: finding.evidence.length })}
        </button>
      {/if}
      {#if action}<div class="action">{@render action()}</div>{/if}
    </div>
    {#if open}
      <dl class="evidence">
        {#each finding.evidence as e, i (i)}
          <dt>{t(e.label)}</dt>
          <dd class="mono">{t(e.value)}</dd>
        {/each}
      </dl>
    {/if}
  </div>
</article>

<style>
  .finding {
    position: relative;
    display: flex;
    gap: calc(var(--space) * 4);
    padding: calc(var(--space) * 4) calc(var(--space) * 5);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-card);
    transition: border-color var(--dur) var(--ease), background-color var(--dur) var(--ease);
  }
  .finding:hover {
    border-color: var(--border-strong);
    background: var(--bg-card-hover);
  }
  .bar {
    flex-shrink: 0;
    width: 3px;
    border-radius: 3px;
    background: var(--info);
  }
  .high .bar {
    background: var(--danger);
  }
  .moderate .bar {
    background: var(--warn);
  }
  .main {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .top {
    display: flex;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
    align-items: flex-start;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    flex-shrink: 0;
  }
  .detail {
    color: var(--text-muted);
    font-size: 13px;
  }
  .advice {
    font-size: 13px;
  }
  .advice span {
    color: var(--accent-from);
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
  }
  .ev-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
  }
  .ev-toggle:hover {
    color: var(--text);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .evidence {
    display: grid;
    grid-template-columns: minmax(160px, max-content) 1fr;
    gap: 4px calc(var(--space) * 4);
    margin: 0;
    padding: calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-size: 12px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    word-break: break-word;
  }
</style>
