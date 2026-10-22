<script lang="ts">
  import type { Snippet } from "svelte";

  // Каркас екрана: заголовок + підзаголовок, прокручуваний вміст, sticky-панель дій унизу
  // (ліворуч — вторинні дії, праворуч — головна).
  let {
    title,
    subtitle,
    children,
    secondary,
    primary,
  }: {
    title: string;
    subtitle?: string;
    children: Snippet;
    secondary?: Snippet;
    primary?: Snippet;
  } = $props();
</script>

<div class="page">
  <div class="scroll">
    <header>
      <h1>{title}</h1>
      {#if subtitle}<p class="muted">{subtitle}</p>{/if}
    </header>
    <div class="content">{@render children()}</div>
  </div>
  {#if secondary || primary}
    <footer class="actionbar">
      <div class="left">{#if secondary}{@render secondary()}{/if}</div>
      <div class="right">{#if primary}{@render primary()}{/if}</div>
    </footer>
  {/if}
</div>

<style>
  .page {
    position: relative;
    z-index: 1;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    animation: page-in var(--dur-slow) var(--ease);
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: calc(var(--space) * 8) calc(var(--space) * 8) calc(var(--space) * 6);
  }
  header {
    display: grid;
    gap: calc(var(--space) * 1.5);
    margin-bottom: calc(var(--space) * 6);
    max-width: 1100px;
  }
  .content {
    display: grid;
    gap: calc(var(--space) * 5);
    max-width: 1100px;
  }
  .actionbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 3) calc(var(--space) * 8);
    border-top: 1px solid var(--border);
    /* Без backdrop-filter: над анімованим фоном розмиття перераховувалося б щокадру. */
    background: color-mix(in srgb, var(--bg-sidebar) 94%, transparent);
  }
  .left,
  .right {
    display: flex;
    gap: calc(var(--space) * 2);
    align-items: center;
  }
</style>
