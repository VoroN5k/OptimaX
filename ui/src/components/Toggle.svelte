<script lang="ts">
  let {
    checked = $bindable(false),
    label,
    disabled = false,
    onchange,
  }: { checked?: boolean; label: string; disabled?: boolean; onchange?: (v: boolean) => void } = $props();

  function toggle() {
    if (disabled) return;
    checked = !checked;
    onchange?.(checked);
  }
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  class="toggle"
  class:on={checked}
  {disabled}
  onclick={toggle}
>
  <span class="track"><span class="knob"></span></span>
</button>

<style>
  /* Кнопка — ціль 32 px заввишки (docs/DESIGN.md розд. 10), сама доріжка — 24 px. */
  .toggle {
    display: inline-flex;
    align-items: center;
    width: 42px;
    height: 32px;
    flex-shrink: 0;
    border: 0;
    border-radius: var(--radius-pill);
    background: transparent;
    cursor: pointer;
    padding: 0;
  }
  .track {
    position: relative;
    display: block;
    width: 42px;
    height: 24px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    transition: background-color var(--dur) var(--ease), border-color var(--dur) var(--ease);
  }
  .toggle.on .track {
    background: var(--accent-gradient);
    border-color: transparent;
  }
  .toggle:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--text);
    transition: transform var(--dur) var(--ease);
  }
  .on .knob {
    transform: translateX(18px);
    background: var(--onA);
  }
</style>
