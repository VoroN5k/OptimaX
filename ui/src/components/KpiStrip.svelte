<script lang="ts" module>
  /** Клітинка смуги показників. */
  export type Kpi = { label: string; value: string | null; unit?: string; tone?: "warn" | "ok" | "bad" };
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  // Смуга ключових показників (docs/DESIGN.md розд. 6.2): одна плитка, клітинки рівно діляться
  // (auto-fit, від 180px), між ними межа. Значення `null` — «—» (немає даних: нічого не вигадуємо).
  // Прожектор іде за курсором (лише в режимі «Повні»), блиск проходить раз на 6 с.
  let { items, label, footer }: { items: Kpi[]; label: string; footer?: Snippet } = $props();

  let mx = $state(-999);
  let my = $state(-999);
  function onmousemove(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    mx = e.clientX - r.left;
    my = e.clientY - r.top;
  }
</script>

<section class="tile strip" aria-label={label} {onmousemove} style:--mx="{mx}px" style:--my="{my}px">
  <div class="spot" aria-hidden="true"></div>
  <div class="shimmer ambient" aria-hidden="true"></div>
  <div class="cells">
    {#each items as k (k.label)}
      <div class="cell">
        <div class="micro">{k.label}</div>
        <div class="value num {k.tone ?? ''}" class:empty={k.value === null}>
          {k.value ?? "—"}{#if k.value !== null && k.unit}<span class="unit">{k.unit}</span>{/if}
        </div>
      </div>
    {/each}
  </div>
  {#if footer}<div class="footer small">{@render footer()}</div>{/if}
</section>

<style>
  .strip {
    overflow: hidden;
  }
  .spot {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: radial-gradient(380px circle at var(--mx) var(--my), rgba(var(--glow), 0.13), transparent 60%);
  }
  .shimmer {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 30%;
    left: 0;
    pointer-events: none;
    background: var(--shine-soft);
    /* Лише transform (окремий шар композиції): анімація `left` перераховувала розкладку щокадру. */
    transform: translateX(-133%) skewX(-18deg);
    will-change: transform;
    animation: shimmer 6s ease-in-out 1.5s infinite;
  }
  /* Від -40 % до 140 % ширини смуги; ширина відблиску — 30 % смуги, тож у власних відсотках. */
  @keyframes shimmer {
    0% {
      transform: translateX(-133%) skewX(-18deg);
    }
    35%,
    100% {
      transform: translateX(467%) skewX(-18deg);
    }
  }
  .cells {
    position: relative;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(180px, 100%), 1fr));
  }
  .cell {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: calc(var(--space) * 4) calc(var(--space) * 5.5);
  }
  .cell + .cell {
    border-left: 1px solid var(--line-soft);
  }
  .value {
    font-size: var(--fs-kpi);
    line-height: var(--lh-kpi);
    font-weight: 300;
    letter-spacing: -0.03em;
  }
  .value.empty {
    color: var(--text-3);
  }
  .value.warn {
    color: var(--warn);
  }
  .value.ok {
    color: var(--ok);
  }
  .value.bad {
    color: var(--bad);
  }
  .unit {
    margin-left: 4px;
    font-size: 14px;
    letter-spacing: 0;
    color: var(--text-2);
  }
  .footer {
    position: relative;
    display: flex;
    flex-wrap: wrap;
    gap: 6px calc(var(--space) * 6);
    padding: calc(var(--space) * 2.5) calc(var(--space) * 5.5);
    border-top: 1px solid var(--line-soft);
    color: var(--text-2);
  }
  :global(:root[data-effects="calm"]) .strip :is(.spot, .shimmer),
  :global(:root[data-effects="off"]) .strip :is(.spot, .shimmer),
  :global(:root.measuring) .strip :is(.spot, .shimmer) {
    display: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .shimmer {
      display: none;
    }
  }
</style>
