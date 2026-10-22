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
  <span class="knob"></span>
</button>

<style>
  .toggle {
    position: relative;
    width: 42px;
    height: 24px;
    flex-shrink: 0;
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    cursor: pointer;
    padding: 0;
    transition: background-color var(--dur) var(--ease), border-color var(--dur) var(--ease);
  }
  .toggle.on {
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
  }
</style>
