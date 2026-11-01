<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  // Розкривна картка: заголовок і короткий підсумок видно завжди, вміст — за кліком.
  // `keep` — вміст лишається змонтованим і коли згорнуто (для компонентів зі своїм станом).
  let {
    title,
    summary,
    open = $bindable(false),
    keep = false,
    children,
  }: {
    title: string;
    summary?: string;
    open?: boolean;
    keep?: boolean;
    children: Snippet;
  } = $props();

  const id = `fold-${Math.random().toString(36).slice(2, 9)}`;
</script>

<section class="fold" class:open>
  <button type="button" class="head" aria-expanded={open} aria-controls={id} onclick={() => (open = !open)}>
    <span class="chev" aria-hidden="true"><Icon name="chevron" size={16} /></span>
    <span class="text">
      <span class="title">{title}</span>
      {#if summary}<span class="summary small muted">{summary}</span>{/if}
    </span>
  </button>
  {#if open || keep}
    <div class="body" {id} hidden={!open}>
      {@render children()}
    </div>
  {/if}
</section>

<style>
  .fold {
    background:
      linear-gradient(180deg, var(--card-sheen), transparent 40%),
      var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    transition: border-color var(--dur) var(--ease);
  }
  .fold:hover {
    border-color: var(--border-strong);
  }
  .head {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    width: 100%;
    padding: calc(var(--space) * 4) calc(var(--space) * 5);
    border: none;
    border-radius: var(--radius-lg);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .head:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
  }
  .chev {
    display: inline-flex;
    color: var(--text-muted);
    transition: transform var(--dur) var(--ease);
  }
  .open .chev {
    transform: rotate(90deg);
  }
  .text {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: calc(var(--space) * 1) calc(var(--space) * 3);
    min-width: 0;
  }
  .title {
    font-size: 16px;
    font-weight: 600;
  }
  .body {
    padding: 0 calc(var(--space) * 5) calc(var(--space) * 5);
  }
  .body[hidden] {
    display: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .chev {
      transition: none;
    }
  }
</style>
