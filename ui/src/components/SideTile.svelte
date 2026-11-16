<script lang="ts">
  import type { Snippet } from "svelte";
  import { tk } from "../lib/i18n";

  // Бічна плитка знахідки (docs/DESIGN.md розд. 7, SideTile): пілюля вердикту, впевненість,
  // заголовок 16/22. Уся плитка — кнопка (якщо є `onclick`), з підйомом на hover.
  let {
    title,
    confidence,
    pill,
    onclick,
    children,
  }: {
    title: string;
    /** Впевненість, % (null — не показуємо). */
    confidence?: number | null;
    pill: Snippet;
    onclick?: () => void;
    children?: Snippet;
  } = $props();
</script>

{#snippet body()}
  <div class="head">
    {@render pill()}
    {#if confidence != null}
      <span class="conf small">{tk("ui.finding.confidence", { v: confidence })}</span>
    {/if}
  </div>
  <div class="title">{title}</div>
  {#if children}<div class="more small">{@render children()}</div>{/if}
{/snippet}

{#if onclick}
  <button type="button" class="tile lift side" {onclick}>{@render body()}</button>
{:else}
  <div class="tile side">{@render body()}</div>
{/if}

<style>
  .side {
    display: flex;
    flex-direction: column;
    gap: calc(var(--space) * 3);
    width: 100%;
    padding: calc(var(--space) * 4.5) calc(var(--space) * 5);
    color: inherit;
    font: inherit;
    text-align: left;
  }
  button.side {
    cursor: pointer;
  }
  button.side:active {
    transform: scale(0.98);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 2);
  }
  .conf {
    color: var(--text-2);
  }
  .title {
    font-size: var(--fs-tile);
    line-height: var(--lh-tile);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .more {
    color: var(--text-2);
  }
</style>
