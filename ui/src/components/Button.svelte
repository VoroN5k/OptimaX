<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    variant = "secondary",
    icon,
    loading = false,
    disabled = false,
    small = false,
    label,
    onclick,
    children,
  }: {
    variant?: "primary" | "secondary" | "danger" | "ghost";
    icon?: string;
    loading?: boolean;
    disabled?: boolean;
    small?: boolean;
    /** Обов'язково для кнопки лише з іконкою. */
    label?: string;
    onclick?: () => void;
    children?: Snippet;
  } = $props();
</script>

<button
  type="button"
  class="btn {variant}"
  class:small
  class:loading
  disabled={disabled || loading}
  aria-busy={loading}
  aria-label={label}
  {onclick}
>
  {#if loading}
    <span class="spinner" aria-hidden="true"></span>
  {:else if icon}
    <Icon name={icon} size={small ? 15 : 17} />
  {/if}
  {#if children}<span>{@render children()}</span>{/if}
</button>

<style>
  /* Кнопки (docs/DESIGN.md розд. 7, Button): головна — градієнт акценту, текст --onA, світіння,
     на hover підйом на 1px і відблиск; другорядна — прозора з межею; натиск — scale(.98). */
  .btn {
    position: relative;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: calc(var(--space) * 2);
    height: var(--ctl-h-lg);
    padding: 0 calc(var(--space) * 4.5);
    border-radius: var(--radius-md);
    border: 1px solid var(--line-ctl);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--fs-body);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition:
      transform var(--dur-fast) var(--ease),
      box-shadow var(--dur-fast) var(--ease),
      background-color var(--dur-fast) var(--ease),
      border-color var(--dur-fast) var(--ease),
      opacity var(--dur-fast) var(--ease);
  }
  .small {
    height: var(--ctl-h-sm);
    padding: 0 calc(var(--space) * 3);
    border-radius: var(--radius-ctl);
    font-size: 13px;
  }
  .btn:hover:not(:disabled) {
    background: var(--fill-3);
    border-color: var(--line-strong);
  }
  .btn:active:not(:disabled) {
    transform: scale(0.98);
  }
  .btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .primary {
    border: none;
    padding: 0 calc(var(--space) * 5.5);
    background: var(--accent-gradient);
    color: var(--onA);
    font-weight: 600;
    box-shadow: var(--shadow-btn);
  }
  .primary.small {
    padding: 0 calc(var(--space) * 3.5);
  }
  .primary:hover:not(:disabled) {
    background: var(--accent-gradient);
    transform: translateY(-1px);
    box-shadow: var(--shadow-btn-hover);
  }
  .primary:active:not(:disabled) {
    transform: scale(0.98);
  }
  /* Відблиск, що проходить кнопкою на hover. */
  .primary::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: -60%;
    width: 40%;
    background: var(--shine);
    transform: skewX(-20deg);
    transition: left var(--dur-slow) var(--ease);
    pointer-events: none;
  }
  .primary:hover:not(:disabled)::after {
    left: 130%;
  }
  .danger {
    border-color: color-mix(in srgb, var(--bad) 55%, transparent);
    color: var(--bad);
  }
  .danger:hover:not(:disabled) {
    border-color: var(--bad);
    background: color-mix(in srgb, var(--bad) 12%, transparent);
  }
  .ghost {
    border-color: transparent;
    background: transparent;
    color: var(--text-2);
  }
  .ghost:hover:not(:disabled) {
    color: var(--text);
    border-color: transparent;
  }
  .spinner {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-right-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .primary::after {
      display: none;
    }
  }
</style>