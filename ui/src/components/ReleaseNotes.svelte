<script lang="ts">
  import type { NoteKind } from "../lib/bindings/NoteKind";
  import type { NoteSection } from "../lib/bindings/NoteSection";
  import { tk } from "../lib/i18n";

  // Опис оновлення: розділи й пункти вже розібрані бекендом. Текст показується лише як текст
  // (жодного HTML з мережі). «Коротко» — першим, технічні подробиці — за кнопкою.
  let { sections }: { sections: NoteSection[] } = $props();

  const summary = $derived(sections.find((s) => s.kind === "summary"));
  const details = $derived(sections.filter((s) => s.kind !== "summary"));
  let expanded = $state(false);
  const showDetails = $derived(expanded || !summary);
  const detailCount = $derived(details.reduce((n, s) => n + s.items.length, 0));

  const tone: Record<NoteKind, string> = {
    summary: "accent",
    security: "info",
    added: "ok",
    fixed: "warn",
    changed: "info",
    other: "dim",
  };
</script>

{#snippet section(s: NoteSection)}
  <section class="sec">
    {#if s.title && s.kind !== "summary"}
      <h4 class="badge {tone[s.kind]}"><i class="dot" aria-hidden="true"></i>{s.title}</h4>
    {/if}
    <ul class:summary={s.kind === "summary"}>
      {#each s.items as item, i (i)}
        <li class:sub={item.depth > 0}>
          {#each item.spans as sp, j (j)}{#if sp.kind === "bold"}<strong>{sp.text}</strong>{:else if sp.kind === "code"}<code>{sp.text}</code>{:else}{sp.text}{/if}{/each}
        </li>
      {/each}
    </ul>
  </section>
{/snippet}

<div class="notes">
  {#if summary}{@render section(summary)}{/if}
  {#if showDetails}
    {#each details as s, i (i)}{@render section(s)}{/each}
  {/if}
  {#if summary && detailCount}
    <button type="button" class="more small" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
      {expanded ? tk("ui.notes.less") : tk("ui.notes.more", { n: detailCount })}
    </button>
  {/if}
</div>

<style>
  .notes {
    display: grid;
    gap: calc(var(--space) * 3);
    max-height: 320px;
    overflow: auto;
    padding: calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  .sec {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  h4 {
    margin: 0;
  }
  .badge {
    justify-self: start;
    display: inline-flex;
    align-items: center;
    gap: calc(var(--space) * 1.5);
    padding: 2px 10px;
    border-radius: 999px;
    border: 1px solid currentColor;
    font-size: 12px;
    font-weight: 600;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }
  .ok {
    color: var(--ok);
  }
  .warn {
    color: var(--warn);
  }
  .info {
    color: var(--info);
  }
  .dim {
    color: var(--text-muted);
  }
  ul {
    margin: 0;
    padding-left: calc(var(--space) * 5);
    display: grid;
    gap: calc(var(--space) * 1.5);
    font-size: 13px;
    line-height: 1.5;
  }
  ul.summary {
    font-size: 14px;
  }
  li.sub {
    margin-left: calc(var(--space) * 4);
    color: var(--text-muted);
  }
  code {
    font-family: "JetBrains Mono", monospace;
    font-size: 12px;
    padding: 0 4px;
    border-radius: 4px;
    background: var(--bg-card);
    border: 1px solid var(--border);
  }
  .more {
    justify-self: start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-from);
    cursor: pointer;
  }
  .more:hover {
    text-decoration: underline;
  }
  .more:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
    border-radius: 4px;
  }
</style>
