<script lang="ts">
  // Повзунок 0…1 (або іншого діапазону): заповнення градієнтом акценту, значення моноширинним.
  let {
    value = $bindable(0),
    min = 0,
    max = 1,
    step = 0.05,
    label,
    format,
    onchange,
  }: {
    value?: number;
    min?: number;
    max?: number;
    step?: number;
    label: string;
    format?: (v: number) => string;
    onchange?: (v: number) => void;
  } = $props();

  const fill = $derived(max > min ? ((value - min) / (max - min)) * 100 : 0);
</script>

<div class="slider">
  <input
    type="range"
    {min}
    {max}
    {step}
    bind:value
    aria-label={label}
    aria-valuetext={format ? format(value) : String(value)}
    style:--fill="{fill}%"
    onchange={() => onchange?.(value)}
  />
  <span class="mono small value">{format ? format(value) : value}</span>
</div>

<style>
  .slider {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
  input {
    flex: 1;
    height: 6px;
    appearance: none;
    border-radius: 999px;
    background: linear-gradient(90deg, var(--accent-from), var(--accent-to)) 0 / var(--fill) 100% no-repeat,
      var(--bg-input);
    border: 1px solid var(--border);
    cursor: pointer;
  }
  input::-webkit-slider-thumb {
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--text);
    box-shadow: 0 0 0 3px var(--accent-glow);
    transition: transform var(--dur-fast) var(--ease);
  }
  input:active::-webkit-slider-thumb {
    transform: scale(0.9);
  }
  input:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 4px;
  }
  .value {
    min-width: 40px;
    text-align: right;
    color: var(--text-muted);
  }
  @media (prefers-reduced-motion: reduce) {
    input::-webkit-slider-thumb {
      transition: none;
    }
  }
</style>
