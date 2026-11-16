<script lang="ts">
  import type { Snippet } from "svelte";

  // Доріжка графіка шкали запису (docs/DESIGN.md розд. 7, Track): підпис 132px ліворуч, доріжка
  // фіксованої висоти (84 / 64 / 44 / 24 / 20), радіус 10, фон --track. Вміст (SVG чи смуги)
  // дає екран із реальних даних; доріжка відкривається зліва направо (одноразово).
  let {
    label,
    sub,
    height,
    aria,
    children,
  }: {
    label: string;
    sub?: string;
    height: 84 | 64 | 44 | 24 | 20;
    /** Що показує доріжка (і чи є там ривок) — для читачів екрана. */
    aria: string;
    children: Snippet;
  } = $props();
</script>

<div class="track-row">
  <div class="labels">
    <span class="label">{label}</span>
    {#if sub}<span class="sub">{sub}</span>{/if}
  </div>
  <div class="track" style:height="{height}px" role="img" aria-label={aria}>
    <div class="reveal">{@render children()}</div>
  </div>
</div>

<style>
  .track-row {
    display: grid;
    grid-template-columns: 132px minmax(0, 1fr);
    gap: calc(var(--space) * 4);
    align-items: center;
  }
  .labels {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .label {
    font-size: 13px;
    font-weight: 600;
  }
  .sub {
    font-size: var(--fs-micro);
    color: var(--text-3);
  }
  .track {
    position: relative;
    border-radius: var(--radius-ctl);
    background: var(--track);
    overflow: hidden;
  }
  .reveal {
    position: absolute;
    inset: 0;
    animation: reveal 2s var(--ease) 0.7s backwards;
  }
  @keyframes reveal {
    from {
      clip-path: inset(0 100% 0 0);
    }
  }
  /* Лінії графіків не товщають при розтягуванні SVG. */
  .track :global(svg) {
    display: block;
    width: 100%;
    height: 100%;
  }
  .track :global(svg :is(path, line, polyline)) {
    vector-effect: non-scaling-stroke;
  }
  :global(:root[data-effects="off"]) .reveal {
    animation: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .reveal {
      animation: none;
    }
  }
</style>
