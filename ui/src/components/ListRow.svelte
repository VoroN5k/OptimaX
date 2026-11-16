<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  // Рядок розбору (docs/DESIGN.md розд. 7, ListRow): назва 14/600, короткий підсумок, стрілка;
  // увесь рядок — кнопка. Для списку в плитці (межі між рядками дає плитка-контейнер).
  let { title, onclick, children }: { title: string; onclick: () => void; children?: Snippet } = $props();
</script>

<button type="button" class="row" {onclick}>
  <span class="text">
    <span class="title">{title}</span>
    {#if children}<span class="summary small">{@render children()}</span>{/if}
  </span>
  <span class="arrow" aria-hidden="true"><Icon name="chevron" size={12} /></span>
</button>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
    width: 100%;
    min-height: 44px;
    padding: calc(var(--space) * 3) calc(var(--space) * 5);
    border: 0;
    border-top: 1px solid var(--line-soft);
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--dur-fast) var(--ease);
  }
  .row:hover {
    background: var(--hover);
  }
  .row:active {
    transform: scale(0.99);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title {
    font-size: var(--fs-body);
    font-weight: 600;
  }
  .summary {
    color: var(--text-2);
  }
  .arrow {
    display: inline-flex;
    color: var(--text-3);
  }
</style>
