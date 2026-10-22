<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    subtitle,
    actions,
    children,
    padded = true,
  }: {
    title?: string;
    subtitle?: string;
    actions?: Snippet;
    children: Snippet;
    padded?: boolean;
  } = $props();
</script>

<section class="card" class:padded>
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
  .card {
    position: relative;
    background:
      linear-gradient(180deg, var(--card-sheen), transparent 40%),
      var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    transition: border-color var(--dur) var(--ease);
  }
  .card:hover {
    border-color: var(--border-strong);
  }
  .padded {
    padding: calc(var(--space) * 5);
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: calc(var(--space) * 4);
    margin-bottom: calc(var(--space) * 4);
  }
  .actions {
    display: flex;
    gap: calc(var(--space) * 2);
    flex-shrink: 0;
  }
</style>
