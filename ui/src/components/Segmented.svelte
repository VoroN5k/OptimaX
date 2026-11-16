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
  /* Сегментований вибір (docs/DESIGN.md розд. 6.1): той самий вигляд, що й підвкладки. */
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
    padding: 0 calc(var(--space) * 3.5);
    border: none;
    border-radius: var(--radius-ctl);
    background: transparent;
    color: var(--text-2);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition:
      color var(--dur-fast) var(--ease),
      background-color var(--dur-fast) var(--ease),
      transform var(--dur-press) var(--ease);
  }
  button:hover {
    color: var(--text);
    background: var(--hover);
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