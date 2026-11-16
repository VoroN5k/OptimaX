<script lang="ts" module>
  /** Кнопка групи. `active` — виділена дія (напр. «Найгірший ривок»). */
  export type ToolItem = {
    label: string;
    onclick: () => void;
    /** Для кнопок-символів (+, −). */
    aria?: string;
    active?: boolean;
    disabled?: boolean;
    /** Квадратна кнопка для одного символу. */
    square?: boolean;
  };
</script>

<script lang="ts">
  // Група кнопок у спільній рамці (docs/DESIGN.md розд. 7, ToolbarGroup): радіус 10, межа,
  // кнопки 34px, розділювачі 1px. Активна дія — помаранчевий фон і жирніший текст.
  let { items, label }: { items: ToolItem[]; label: string } = $props();
</script>

<div class="group" role="group" aria-label={label}>
  {#each items as it (it.label)}
    <button
      type="button"
      class:active={it.active}
      class:square={it.square}
      aria-label={it.aria}
      aria-pressed={it.active ? true : undefined}
      disabled={it.disabled}
      onclick={it.onclick}>{it.label}</button
    >
  {/each}
</div>

<style>
  .group {
    display: inline-flex;
    border: 1px solid var(--line-ctl);
    border-radius: var(--radius-ctl);
    overflow: hidden;
  }
  button {
    height: var(--ctl-h-sm);
    padding: 0 calc(var(--space) * 3);
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease),
      transform var(--dur-press) var(--ease);
  }
  button + button {
    border-left: 1px solid var(--line-ctl);
  }
  .square {
    width: 38px;
    padding: 0;
    font-size: 16px;
  }
  button:hover:not(:disabled) {
    background: var(--fill-3);
  }
  button:active:not(:disabled) {
    transform: scale(0.98);
  }
  button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  button:focus-visible {
    outline-offset: -2px;
  }
  .active {
    background: color-mix(in srgb, var(--warn) 12%, transparent);
    color: var(--warn);
    font-weight: 600;
  }
</style>
