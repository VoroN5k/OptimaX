<script lang="ts">
  import type { Plan } from "../lib/bindings/Plan";
  import { t, tk } from "../lib/i18n";
  import Icon from "./Icon.svelte";

  // «Що зміниться»: місце, старе → нове значення моноширинним шрифтом, вплив.
  let { plan, open = $bindable(true) }: { plan: Plan; open?: boolean } = $props();
</script>

<div class="preview">
  <button type="button" class="toggle" aria-expanded={open} onclick={() => (open = !open)}>
    <span class="chev" class:open><Icon name="chevron" size={15} /></span>
    {tk("ui.preview.title")}
  </button>
  {#if open}
    <div class="body">
      {#if plan.changes.length === 0}
        <p class="muted">{tk("ui.preview.noop")}</p>
      {/if}
      {#each plan.changes as ch, i (i)}
        <div class="change">
          <div class="row"><span class="k muted small">{tk("ui.preview.where")}</span><code>{ch.target.kind === "registry" ? `${ch.target.hive === "LocalMachine" ? "HKLM" : "HKCU"}\\${ch.target.path}\\${ch.target.name}` : t({ key: `ui.target.${ch.target.kind}`, args: {} })}</code></div>
          <div class="row">
            <span class="k muted small">{tk("ui.preview.value")}</span>
            <code class="old">{t(ch.old_display)}</code>
            <span class="arrow" aria-hidden="true">→</span>
            <code class="new">{t(ch.new_display)}</code>
          </div>
        </div>
      {/each}
      <dl>
        <dt>{tk("ui.preview.performance")}</dt>
        <dd>{t(plan.info.performance)}</dd>
        <dt>{tk("ui.preview.heat")}</dt>
        <dd>{t(plan.info.heat)}</dd>
        <dt>{tk("ui.preview.safety")}</dt>
        <dd>{t(plan.info.safety)}</dd>
      </dl>
    </div>
  {/if}
</div>

<style>
  .preview {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    width: 100%;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: none;
    background: transparent;
    cursor: pointer;
    font-weight: 600;
    text-align: left;
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .body {
    padding: 0 calc(var(--space) * 4) calc(var(--space) * 4);
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .change {
    display: grid;
    gap: calc(var(--space) * 1.5);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .k {
    width: 84px;
  }
  code {
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    background: var(--bg-card);
    border: 1px solid var(--border);
    word-break: break-all;
  }
  .old {
    color: var(--text-muted);
  }
  .new {
    color: var(--ok);
  }
  .arrow {
    color: var(--text-dim);
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 1.5) calc(var(--space) * 4);
    margin: 0;
  }
  dt {
    color: var(--text-muted);
    font-size: 12px;
  }
  dd {
    margin: 0;
    font-size: 13px;
  }
</style>
