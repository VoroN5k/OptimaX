<script lang="ts">
  import type { Item } from "../lib/bindings/Item";
  import { size } from "../lib/format";
  import { tk } from "../lib/i18n";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import VerdictBadge from "./VerdictBadge.svelte";

  // Рядок очищення: вердикт визначив бекенд; тут лише показ. 🔴 і ⚪ не мають чекбокса.
  let {
    item,
    checked = false,
    disabled = false,
    ontoggle,
    onreveal,
    ondism,
    onhibernate,
  }: {
    item: Item;
    checked?: boolean;
    disabled?: boolean;
    ontoggle?: (on: boolean) => void;
    onreveal: () => void;
    ondism?: () => void;
    onhibernate?: () => void;
  } = $props();

  let open = $state(false);
  const id = `row-${Math.random().toString(36).slice(2, 8)}`;
  const partial = $derived(item.selectable && item.eligible_bytes < item.bytes);
</script>

<div class="row" class:checked>
  <div class="check">
    {#if item.selectable}
      <input
        type="checkbox"
        id={id}
        {checked}
        {disabled}
        aria-label={tk("ui.clean.select", { path: item.path })}
        onchange={(e) => ontoggle?.((e.currentTarget as HTMLInputElement).checked)}
      />
    {/if}
  </div>
  <div class="body">
    <div class="top">
      <label class="path mono" for={item.selectable ? id : undefined} title={item.path}>{item.path}</label>
      <span class="size mono">{size(item.selectable ? item.eligible_bytes : item.bytes)}</span>
      <VerdictBadge verdict={item.verdict} />
    </div>
    {#if item.remainder}
      <p class="small dim">{tk("ui.clean.remainder")}</p>
    {/if}
    {#if partial}
      <p class="small dim">{tk("ui.clean.partial", { eligible: size(item.eligible_bytes), total: size(item.bytes) })}</p>
    {/if}
    {#if item.rule}
      <p class="what">{item.rule.description}</p>
      <p class="after small"><span class="muted">{tk("ui.clean.after")}</span> {item.rule.consequences}</p>
      {#if item.rule.manual_hint && !item.selectable}
        <p class="hint small"><span aria-hidden="true">→</span> {item.rule.manual_hint}</p>
      {/if}
    {:else if item.zone}
      <p class="what">{tk(`ui.clean.zone.${item.zone}`)}</p>
    {/if}
    {#if item.unreadable_dirs > 0}
      <p class="small warn">{tk("ui.clean.unreadable", { n: item.unreadable_dirs })}</p>
    {/if}
    <div class="foot">
      {#if item.rule}
        <button type="button" class="toggle small" aria-expanded={open} onclick={() => (open = !open)}>
          <span class="chev" class:open><Icon name="chevron" size={14} /></span>{tk("ui.clean.details")}
        </button>
      {/if}
      <div class="actions">
        {#if item.method === "dism" && ondism}
          <Button small variant="secondary" icon="play" onclick={ondism}>{tk("ui.clean.dism")}</Button>
        {/if}
        {#if item.method === "powercfg" && onhibernate}
          <Button small variant="secondary" icon="play" onclick={onhibernate}>{tk("ui.clean.hibernate_off")}</Button>
        {/if}
        <Button small variant="ghost" icon="folder" onclick={onreveal}>{tk("ui.clean.reveal")}</Button>
      </div>
    </div>
    {#if open && item.rule}
      <dl class="details">
        <dt>{tk("ui.clean.rule_id")}</dt><dd class="mono">{item.rule.id}</dd>
        <dt>{tk("ui.clean.regenerates")}</dt><dd>{item.rule.regenerates ? tk("ui.common.yes") : tk("ui.common.no")}</dd>
        <dt>{tk("ui.clean.files")}</dt><dd class="mono">{item.selectable ? `${item.eligible_files} / ${item.files}` : item.files}</dd>
        <dt>{tk("ui.clean.source")}</dt><dd class="src">{item.rule.source}</dd>
        <dt>{tk("ui.clean.verified_on")}</dt><dd>{item.rule.verified_on}</dd>
      </dl>
    {/if}
  </div>
</div>

<style>
  .row {
    display: flex;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    transition: border-color var(--dur) var(--ease);
  }
  .row:hover {
    border-color: var(--border-strong);
  }
  .row.checked {
    border-color: color-mix(in srgb, var(--accent-from) 55%, var(--border));
  }
  .check {
    width: 18px;
    flex-shrink: 0;
    padding-top: 2px;
  }
  input[type="checkbox"] {
    width: 16px;
    height: 16px;
    margin: 0;
    accent-color: var(--accent-from);
    cursor: pointer;
  }
  .body {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: calc(var(--space) * 1.5);
  }
  .top {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    min-width: 0;
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  label.path[for] {
    cursor: pointer;
  }
  .size {
    flex-shrink: 0;
    font-weight: 600;
  }
  p {
    margin: 0;
  }
  .what {
    color: var(--text);
  }
  .after,
  .hint {
    color: var(--text-muted);
  }
  .warn {
    color: var(--warn);
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 2);
  }
  .actions {
    display: flex;
    gap: calc(var(--space) * 2);
    margin-left: auto;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: calc(var(--space) * 1);
    padding: 0;
    border: none;
    background: none;
    color: var(--text-muted);
    cursor: pointer;
  }
  .toggle:hover {
    color: var(--text);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .details {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 1.5) calc(var(--space) * 4);
    margin: 0;
    padding-top: calc(var(--space) * 2);
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    min-width: 0;
  }
  .src {
    overflow-wrap: anywhere;
  }
</style>
