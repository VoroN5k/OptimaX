<script lang="ts">
  // Маркер ривка (docs/DESIGN.md розд. 7, JerkMarker): вертикальна лінія --warn → прозоро і
  // підказка «Ривок · t с · кадр N мс», що з'являється з легким підйомом після відкриття графіка.
  // Позицію й числа дає екран із реальних даних запису.
  let {
    left,
    title,
    detail,
  }: {
    /** CSS-позиція лінії в контейнері шкали (напр. `calc(148px + (100% - 148px) * 0.06)`). */
    left: string;
    title: string;
    /** Час і тривалість кадру (з даних). */
    detail: string;
  } = $props();
</script>

<div class="marker" style:left aria-hidden="true">
  <div class="line"></div>
  <div class="tip"><span class="t">{title}</span> · <span class="mono">{detail}</span></div>
</div>

<style>
  .marker {
    position: absolute;
    top: 0;
    bottom: 30px;
    width: 0;
    pointer-events: none;
  }
  .line {
    position: absolute;
    top: -4px;
    bottom: 0;
    left: -0.5px;
    width: 1px;
    background: linear-gradient(180deg, var(--warn), color-mix(in srgb, var(--warn) 15%, transparent));
  }
  .tip {
    position: absolute;
    top: 4px;
    left: 12px;
    white-space: nowrap;
    border-radius: var(--radius-ctl);
    border: 1px solid color-mix(in srgb, var(--warn) 45%, transparent);
    background: var(--drawer-bg);
    box-shadow: var(--shadow-modal);
    padding: 7px 12px;
    font-size: var(--fs-small);
    line-height: var(--lh-small);
    animation: pop 0.5s var(--ease) 2.2s backwards;
  }
  .t {
    color: var(--warn);
    font-weight: 600;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
  :global(:root[data-effects="off"]) .tip {
    animation: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .tip {
      animation: none;
    }
  }
</style>
