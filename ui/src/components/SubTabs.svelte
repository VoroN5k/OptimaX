<script lang="ts" generics="T extends string">
  // Сегментований перемикач підрозділів у шапці сторінки (docs/DESIGN.md розд. 6.1):
  // role=tablist / tab, стрілки ← → між вкладками.
  let {
    options,
    value,
    label,
    onselect,
  }: {
    options: { value: T; label: string }[];
    value: T;
    label: string;
    onselect: (v: T) => void;
  } = $props();

  let buttons = $state<HTMLButtonElement[]>([]);

  function onkeydown(e: KeyboardEvent) {
    const i = options.findIndex((o) => o.value === value);
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    const next = options[i + step];
    if (step && next) {
      e.preventDefault();
      onselect(next.value);
      buttons[i + step]?.focus();
    }
  }
</script>

<div class="seg" role="tablist" aria-label={label} tabindex="-1" {onkeydown}>
  {#each options as o, i (o.value)}
    <button
      bind:this={buttons[i]}
      type="button"
      role="tab"
      aria-selected={o.value === value}
      tabindex={o.value === value ? 0 : -1}
      class:active={o.value === value}
      onclick={() => onselect(o.value)}>{o.label}</button
    >
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--space);
    padding: var(--space);
    border-radius: var(--radius-seg);
    background: var(--fill-2);
    border: 1px solid var(--line-seg);
  }
  button {
    height: var(--ctl-h-sm);
    padding: 0 calc(var(--space) * 4);
    border: 0;
    border-radius: var(--radius-ctl);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-body);
    font-weight: 500;
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease),
      color var(--dur-fast) var(--ease),
      transform var(--dur-press) var(--ease);
  }
  button:hover {
    background: var(--hover);
    color: var(--text);
  }
  button:active {
    transform: scale(0.98);
  }
  button.active {
    color: var(--text);
    font-weight: 600;
    background: var(--seg-active);
    box-shadow: var(--seg-active-shadow);
  }
</style>
