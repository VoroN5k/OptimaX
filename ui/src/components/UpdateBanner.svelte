<script lang="ts">
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";

  // Закриту плашку для цієї версії більше не показуємо (зручність одного користувача:
  // localStorage може бути недоступний — тоді плашка просто з'явиться знову).
  const KEY = "optimax.update.dismissed";

  function readDismissed(): string | null {
    try {
      return localStorage.getItem(KEY);
    } catch {
      return null;
    }
  }

  let dismissed = $state(readDismissed());
  const version = $derived(ui.update?.available?.version ?? null);
  // На «Налаштуваннях» оновлення й так видно в картці «Оновлення».
  const show = $derived(version !== null && version !== dismissed && ui.screen !== "settings");

  function close() {
    dismissed = version;
    try {
      if (version) localStorage.setItem(KEY, version);
    } catch {
      // без збереження — плашка повернеться при наступному запуску
    }
  }
</script>

{#if show && ui.update?.available}
  <div class="banner" role="status" aria-live="polite">
    <Icon name="refresh" size={17} />
    <p>
      {tk("ui.update.banner", { v: ui.update.available.version, current: ui.update.current })}
    </p>
    <Button small variant="primary" onclick={() => (ui.screen = "settings")}>{tk("ui.update.banner_open")}</Button>
    <Button small variant="ghost" icon="close" label={tk("ui.update.banner_close")} onclick={close} />
  </div>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    margin: calc(var(--space) * 4) calc(var(--space) * 8) 0;
    padding: calc(var(--space) * 2.5) calc(var(--space) * 3) calc(var(--space) * 2.5) calc(var(--space) * 4);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    background: linear-gradient(90deg, var(--accent-glow), transparent 70%), var(--bg-card);
    color: var(--text);
    font-size: 13px;
    animation: in var(--dur) var(--ease);
  }
  .banner :global(svg) {
    flex: none;
    color: var(--accent-from);
  }
  p {
    flex: 1;
    margin: 0;
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(-8px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .banner {
      animation: none;
    }
  }
</style>
