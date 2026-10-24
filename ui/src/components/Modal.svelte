<script lang="ts">
  import type { Snippet } from "svelte";
  import { tk } from "../lib/i18n";
  import Button from "./Button.svelte";

  // Модальне підтвердження для системних/ризикових дій: опис наслідків, Esc — скасувати.
  let {
    title,
    confirmLabel,
    danger = false,
    wide = false,
    busy = false,
    onconfirm,
    oncancel,
    children,
  }: {
    title: string;
    confirmLabel: string;
    danger?: boolean;
    /** Ширше вікно (довгі шляхи, списки). */
    wide?: boolean;
    busy?: boolean;
    onconfirm: () => void;
    oncancel: () => void;
    children: Snippet;
  } = $props();

  let dialog = $state<HTMLDivElement | undefined>();

  $effect(() => {
    const prev = document.activeElement as HTMLElement | null;
    dialog?.querySelector<HTMLElement>("button")?.focus();
    return () => prev?.focus();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !busy) oncancel();
    if (e.key === "Tab" && dialog) {
      // Фокус не виходить за межі вікна.
      const items = [...dialog.querySelectorAll<HTMLElement>("button:not(:disabled), [href], input, select")];
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first?.focus();
      }
    }
  }
</script>

<div class="backdrop" role="presentation" {onkeydown}>
  <div class="dialog" class:wide role="dialog" aria-modal="true" aria-labelledby="modal-title" bind:this={dialog}>
    <h2 id="modal-title">{title}</h2>
    <div class="content">{@render children()}</div>
    <div class="buttons">
      <Button variant="secondary" disabled={busy} onclick={oncancel}>{tk("ui.common.cancel")}</Button>
      <Button variant={danger ? "danger" : "primary"} loading={busy} onclick={onconfirm}>{confirmLabel}</Button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    /* Лише область вмісту: вікно центрується там і не заходить під бічну панель. */
    inset: 0 0 0 var(--sidebar-width);
    z-index: 50;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--bg-app) 70%, transparent);
    backdrop-filter: blur(4px);
    animation: fade var(--dur) var(--ease);
  }
  .dialog {
    width: min(560px, calc(100vw - var(--sidebar-width) - 48px));
    overflow-x: hidden;
    max-height: calc(100vh - 64px);
    overflow-y: auto;
    padding: calc(var(--space) * 6);
    border-radius: var(--radius-lg);
    border: 1px solid var(--border-strong);
    background: var(--bg-card);
    box-shadow: var(--shadow-modal);
    display: grid;
    gap: calc(var(--space) * 4);
    animation: page-in var(--dur) var(--ease);
  }
  .dialog.wide {
    width: min(860px, calc(100vw - var(--sidebar-width) - 48px));
  }
  .content {
    min-width: 0;
    color: var(--text-muted);
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
</style>
