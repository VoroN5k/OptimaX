<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  // Згортка всередині плитки (docs/DESIGN.md розд. 7, Disclosure): details/summary, шеврон
  // повертається на 90° за 250 мс, висота заголовка не менше 44px. `count` — число в дужках.
  let {
    title,
    count,
    open = $bindable(false),
    children,
  }: { title: string; count?: number; open?: boolean; children: Snippet } = $props();
</script>

<details class="disc" bind:open>
  <summary>
    <span class="chev" aria-hidden="true"><Icon name="chevron" size={12} /></span>
    {title}
    {#if count !== undefined}<span class="count mono">({count})</span>{/if}
  </summary>
  <div class="body">{@render children()}</div>
</details>

<style>
  .disc {
    border-top: 1px solid var(--line-seg);
  }
  summary {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2.5);
    min-height: 44px;
    list-style: none;
    cursor: pointer;
    border-radius: var(--radius-ctl);
    font-size: 13px;
    font-weight: 500;
    color: var(--text-2);
    transition:
      background-color var(--dur-fast) var(--ease),
      color var(--dur-fast) var(--ease);
  }
  summary::-webkit-details-marker {
    display: none;
  }
  summary:hover {
    color: var(--text);
    background: var(--hover);
  }
  .chev {
    display: inline-flex;
    transition: transform 250ms var(--ease);
  }
  .disc[open] .chev {
    transform: rotate(90deg);
  }
  .count {
    color: var(--text-3);
  }
  .body {
    padding: var(--space) 0 calc(var(--space) * 2) calc(var(--space) * 5.5);
    font-size: 13px;
    line-height: 20px;
    color: var(--text-2);
  }
  @media (prefers-reduced-motion: reduce) {
    .chev {
      transition: none;
    }
  }
</style>
