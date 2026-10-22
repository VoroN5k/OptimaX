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
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: calc(var(--space) * 2);
    height: 38px;
    padding: 0 calc(var(--space) * 4);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition:
      transform var(--dur-fast) var(--ease),
      box-shadow var(--dur) var(--ease),
      background-color var(--dur) var(--ease),
      border-color var(--dur) var(--ease),
      opacity var(--dur) var(--ease);
  }
  .small {
    height: 30px;
    padding: 0 calc(var(--space) * 3);
    font-size: 13px;
  }
  .btn:hover:not(:disabled) {
    background: var(--bg-card-hover);
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
    background: var(--accent-gradient);
    color: var(--on-accent);
  }
  .primary:hover:not(:disabled) {
    background: var(--accent-gradient);
    box-shadow: 0 0 0 4px var(--accent-glow), 0 6px 24px var(--accent-glow);
  }
  .danger {
    border-color: var(--danger);
    color: var(--danger);
  }
  .danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .ghost {
    border-color: transparent;
    color: var(--text-muted);
  }
  .ghost:hover:not(:disabled) {
    color: var(--text);
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
</style>
