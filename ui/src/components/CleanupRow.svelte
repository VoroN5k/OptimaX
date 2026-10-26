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
    onrevealpath,
    ondism,
    onhibernate,
  }: {
    item: Item;
    checked?: boolean;
    disabled?: boolean;
    ontoggle?: (on: boolean) => void;
    onreveal: () => void;
    onrevealpath?: (path: string) => void;
    ondism?: () => void;
    onhibernate?: () => void;
  } = $props();

  let open = $state(false);
  let subsOpen = $state(false);
  /** Ім'я підтеки відносно рядка (повний шлях — у підказці). */
  const rel = (p: string) => (p.length > item.path.length ? p.slice(item.path.length).replace(/^\\/, "") : p);
  const id = `row-${Math.random().toString(36).slice(2, 8)}`;
  const partial = $derived(item.selectable && item.eligible_bytes < item.bytes);
  // Правило дозволяє чистити, але зараз під умови нічого не підпадає (визначає бекенд).
  const nothingNote = $derived(
    item.rule?.min_age_days != null
      ? tk("ui.clean.nothing_age", { days: item.rule.min_age_days, total: size(item.bytes) })
      : tk("ui.clean.nothing", { total: size(item.bytes) }),
  );
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
    {:else if item.nothing_eligible}
      <input
        type="checkbox"
        disabled
        title={tk("ui.clean.nothing_short")}
        aria-label={tk("ui.clean.nothing_short")}
        aria-describedby="{id}-nothing"
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
      <p class="small dim">{tk("ui.clean.remainder_n", { n: item.small_dirs })}</p>
    {/if}
    {#if item.nothing_eligible}
      <p class="small muted" id="{id}-nothing">{nothingNote}</p>
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
      {#if item.top_subdirs.length}
        <button type="button" class="toggle small" aria-expanded={subsOpen} onclick={() => (subsOpen = !subsOpen)}>
          <span class="chev" class:open={subsOpen}><Icon name="chevron" size={14} /></span>{tk("ui.clean.subdirs", { n: item.top_subdirs.length })}
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
    {#if subsOpen && item.top_subdirs.length}
      <ul class="subs">
        {#each item.top_subdirs as d (d.path)}
          <li>
            <span class="mono sub-name" title={d.path}>{rel(d.path)}</span>
            <span class="mono sub-size">{size(d.bytes)}</span>
            {#if onrevealpath}
              <button
                type="button"
                class="icon-btn"
                aria-label={tk("ui.clean.reveal_path", { path: d.path })}
                title={tk("ui.clean.reveal")}
                onclick={() => onrevealpath?.(d.path)}
              ><Icon name="folder" size={14} /></button>
            {/if}
          </li>
        {/each}
        {#if item.small_dirs > item.top_subdirs.length}
          <li class="dim small">{tk("ui.clean.subdirs_more", { n: Number(item.small_dirs) - item.top_subdirs.length })}</li>
        {/if}
      </ul>
    {/if}
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
  input[type="checkbox"]:disabled {
    cursor: not-allowed;
    opacity: 0.4;
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
  .subs {
    display: grid;
    gap: calc(var(--space) * 1);
    margin: 0;
    padding: calc(var(--space) * 2) 0 0;
    border-top: 1px solid var(--border);
    list-style: none;
    font-size: 12px;
  }
  .subs li {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    min-width: 0;
  }
  .sub-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
  }
  .sub-size {
    flex-shrink: 0;
  }
  .icon-btn {
    display: inline-flex;
    padding: calc(var(--space) * 1);
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
    cursor: pointer;
    transition: color var(--dur-fast) var(--ease);
  }
  .icon-btn:hover {
    color: var(--text);
  }
  .icon-btn:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 1px;
  }
</style>
