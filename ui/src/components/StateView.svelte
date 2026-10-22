<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  // Стани «завантаження / порожньо / помилка» для екранів.
  let {
    kind,
    title,
    text,
    action,
  }: { kind: "loading" | "empty" | "error"; title: string; text?: string; action?: Snippet } = $props();
</script>

<div class="state {kind}" role={kind === "error" ? "alert" : "status"} aria-live="polite">
  {#if kind === "loading"}
    <span class="spinner" aria-hidden="true"></span>
  {:else}
    <span class="icon"><Icon name={kind === "error" ? "warning" : "check"} size={22} /></span>
  {/if}
  <h3>{title}</h3>
  {#if text}<p class="muted small">{text}</p>{/if}
  {#if action}{@render action()}{/if}
</div>

<style>
  .state {
    display: grid;
    justify-items: center;
    gap: calc(var(--space) * 2);
    padding: calc(var(--space) * 10) calc(var(--space) * 4);
    text-align: center;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--info) 14%, transparent);
    color: var(--info);
  }
  .error .icon {
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    color: var(--danger);
  }
  p {
    max-width: 480px;
  }
  .spinner {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 3px solid var(--border-strong);
    border-top-color: var(--accent-from);
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
