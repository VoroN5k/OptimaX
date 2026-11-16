<script lang="ts" module>
  /** Пункт легенди: квадрат (стан) чи лінія (метрика) кольору з токена (напр. `--warn`). */
  export type LegendItem = { kind: "square" | "line"; color: string; label: string };
  export type LegendGroup = { title: string; items: LegendItem[] };
</script>

<script lang="ts">
  // Легенда шкали (docs/DESIGN.md розд. 7, Legend): рядки «МЕТРИКИ» / «СТАНИ ПОТОКІВ»,
  // квадрати 9px радіус 3 для станів, лінії 13×2 для ліній; підписи 12px --text-2.
  let { groups }: { groups: LegendGroup[] } = $props();
</script>

<div class="legend">
  {#each groups as g (g.title)}
    <div class="row">
      <span class="micro title">{g.title}</span>
      {#each g.items as it (it.label)}
        <span class="item">
          <span class={it.kind} style:background="var({it.color})" aria-hidden="true"></span>{it.label}
        </span>
      {/each}
    </div>
  {/each}
</div>

<style>
  .legend {
    display: flex;
    flex-direction: column;
    gap: calc(var(--space) * 3);
    font-size: var(--fs-small);
    color: var(--text-2);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 2) calc(var(--space) * 4.5);
  }
  .title {
    flex: 0 0 104px;
  }
  .item {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }
  .square {
    width: 9px;
    height: 9px;
    border-radius: 3px;
  }
  .line {
    width: 13px;
    height: 2px;
  }
</style>
