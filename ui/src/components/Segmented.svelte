<script lang="ts" generics="T extends string | number">
  let {
    options,
    value = $bindable(),
    label,
    onchange,
  }: {
    options: { value: T; label: string }[];
    value: T;
    label: string;
    onchange?: (v: T) => void;
  } = $props();

  function select(v: T) {
    value = v;
    onchange?.(v);
  }

  function onkeydown(e: KeyboardEvent) {
    const i = options.findIndex((o) => o.value === value);
    const next = e.key === "ArrowRight" ? i + 1 : e.key === "ArrowLeft" ? i - 1 : -1;
    const opt = options[next];
    if (opt) {
      e.preventDefault();
      select(opt.value);
    }
  }
</script>

<div class="seg" role="radiogroup" aria-label={label} tabindex="-1" {onkeydown}>
  {#each options as o (o.value)}
    <button
      type="button"
      role="radio"
      aria-checked={o.value === value}
      tabindex={o.value === value ? 0 : -1}
      class:active={o.value === value}
      onclick={() => select(o.value)}>{o.label}</button
    >
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    padding: 3px;
    gap: 2px;
    border-radius: var(--radius-md);
    background: var(--bg-input);
    border: 1px solid var(--border);
  }
  button {
    border: none;
    background: transparent;
    color: var(--text-muted);
    padding: calc(var(--space) * 1.5) calc(var(--space) * 3.5);
    border-radius: calc(var(--radius-md) - 3px);
    cursor: pointer;
    font-weight: 500;
    font-size: 13px;
    transition: color var(--dur) var(--ease), background-color var(--dur) var(--ease);
  }
  button:hover {
    color: var(--text);
  }
  button.active {
    color: var(--on-accent);
    background: var(--accent-gradient);
  }
</style>
