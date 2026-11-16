<script lang="ts">
  import type { Snippet } from "svelte";

  // Плитка з заголовком (docs/DESIGN.md розд. 7, Tile): скло, межа 1px, радіус 18, відступ 24×28.
  let {
    title,
    subtitle,
    actions,
    children,
    padded = true,
    lift = false,
  }: {
    title?: string;
    subtitle?: string;
    actions?: Snippet;
    children: Snippet;
    padded?: boolean;
    /** Підйом на hover (для плиток, що ведуть кудись). */
    lift?: boolean;
  } = $props();
</script>

<section class="tile card" class:padded class:lift>
  {#if title || actions}
    <header>
      <div>
        {#if title}<h2>{title}</h2>{/if}
        {#if subtitle}<p class="muted small">{subtitle}</p>{/if}
      </div>
      {#if actions}<div class="actions">{@render actions()}</div>{/if}
    </header>
  {/if}
  {@render children()}
</section>

<style>
  .padded {
    padding: calc(var(--space) * 6) calc(var(--space) * 7);
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: calc(var(--space) * 4);
    margin-bottom: calc(var(--space) * 4);
  }
  h2 {
    font-size: var(--fs-tile);
    line-height: var(--lh-tile);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  header p {
    margin-top: 2px;
  }
  .actions {
    display: flex;
    gap: calc(var(--space) * 2);
    flex-shrink: 0;
  }
</style>
