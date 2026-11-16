<script lang="ts">
  import type { Snippet } from "svelte";
  import { tk } from "../lib/i18n";
  import { siblingsOf, tabOf } from "../lib/nav";
  import { ui, type ScreenId } from "../lib/state.svelte";
  import SubTabs from "./SubTabs.svelte";

  // Каркас екрана (docs/DESIGN.md розд. 6): шапка — заголовок і підзаголовок ліворуч, підрозділи
  // вкладки праворуч; прокручуваний вміст до 1176px по центру; sticky-панель дій унизу (ліворуч —
  // вторинні дії, праворуч — головна). `tabs` — власні підрозділи сторінки замість розділів вкладки.
  let {
    title,
    subtitle,
    children,
    secondary,
    primary,
    tabs,
  }: {
    title: string;
    subtitle?: string;
    children: Snippet;
    secondary?: Snippet;
    primary?: Snippet;
    tabs?: Snippet;
  } = $props();

  const siblings = $derived(siblingsOf(ui.screen));
</script>

<div class="page">
  <div class="scroll">
    <header>
      <div class="titles">
        <h1>{title}</h1>
        {#if subtitle}<p class="muted">{subtitle}</p>{/if}
      </div>
      {#if tabs}
        <div class="head-tools">{@render tabs()}</div>
      {:else if siblings.length > 1}
        <SubTabs
          label={tk(`ui.tab.${tabOf(ui.screen)}`)}
          value={ui.screen}
          options={siblings.map((s: ScreenId) => ({ value: s, label: tk(`ui.nav.${s}`) }))}
          onselect={(s) => (ui.screen = s)}
        />
      {/if}
    </header>
    <div class="content">{@render children()}</div>
  </div>
  {#if secondary || primary}
    <footer class="actionbar">
      <div class="bar">
        <div class="left">{#if secondary}{@render secondary()}{/if}</div>
        <div class="right">{#if primary}{@render primary()}{/if}</div>
      </div>
    </footer>
  {/if}
</div>

<style>
  .page {
    position: relative;
    z-index: 1;
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    animation: page-in var(--dur-slow) var(--ease);
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: calc(var(--space) * 7) calc(var(--space) * 8) calc(var(--space) * 8);
  }
  /* Підрозділи завжди праворуч (не переносяться під заголовок залежно від довжини підзаголовка):
     заголовок стискається, перемикач — ні. */
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: calc(var(--space) * 3.5) calc(var(--space) * 6);
    margin-bottom: calc(var(--space) * 5);
  }
  .titles {
    display: grid;
    gap: var(--space);
    flex: 1 1 0;
    /* 200 px: на мінімальному вікні (960) поруч уміщаються 5 підрозділів «Діагностики» й «Довідка». */
    min-width: 200px;
  }
  header > :global(:not(.titles)) {
    flex-shrink: 0;
  }
  .head-tools {
    display: flex;
    align-items: center;
  }
  /* Шапка, вміст і панель дій — одна колонка, по центру на широких екранах. */
  header,
  .content,
  .bar {
    width: 100%;
    max-width: var(--content-width);
    margin-inline: auto;
  }
  .content {
    display: grid;
    gap: calc(var(--space) * 5);
  }
  .bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
  .actionbar {
    padding: calc(var(--space) * 3) calc(var(--space) * 8);
    border-top: 1px solid var(--line-soft);
    /* Без backdrop-filter: над анімованим фоном розмиття перераховувалося б щокадру. */
    background: var(--bar-bg);
  }
  .left,
  .right {
    display: flex;
    gap: calc(var(--space) * 2.5);
    align-items: center;
  }
</style>
