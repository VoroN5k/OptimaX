<script lang="ts">
  import { tick } from "svelte";
  import { log, ui } from "../lib/state.svelte";
  import { time } from "../lib/format";
  import { tk } from "../lib/i18n";

  // Журнал дій — висувна панель справа (кнопка «Журнал» у верхній панелі): моноширинний,
  // автопрокрутка, у DOM не більше 500 рядків (обрізання в state). Esc закриває.
  let box = $state<HTMLDivElement | undefined>();

  $effect(() => {
    const n = log.lines.length;
    if (ui.logOpen && box && n) {
      void tick().then(() => {
        if (box) box.scrollTop = box.scrollHeight;
      });
    }
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && ui.logOpen) ui.logOpen = false;
  }
</script>

<svelte:window {onkeydown} />

{#if ui.logOpen}
  <aside class="drawer" aria-label={tk("ui.log.title")}>
    <div class="head">
      <strong>{tk("ui.log.title")}</strong>
      <span class="small dim">{tk("ui.log.count", { n: log.lines.length })} · v{ui.info?.version ?? "…"}</span>
      <button type="button" class="close" aria-label={tk("ui.common.close")} onclick={() => (ui.logOpen = false)}>✕</button>
    </div>
    <div class="lines mono" bind:this={box} aria-live="polite">
      {#each log.lines as l, i (i)}
        <div class="line {l.level}">
          <span class="dim">{time(l.unix_ms)}</span>
          <span class="mark" aria-hidden="true">{l.level === "info" ? "✓" : l.level === "warn" ? "!" : "✕"}</span>
          <span>{l.message}</span>
        </div>
      {:else}
        <div class="dim">{tk("ui.log.empty")}</div>
      {/each}
    </div>
  </aside>
{/if}

<style>
  .drawer {
    position: fixed;
    z-index: 20;
    top: var(--topbar-h);
    right: 0;
    bottom: 0;
    width: min(480px, 100vw);
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--line);
    background: var(--drawer-bg);
    box-shadow: var(--shadow-modal);
    animation: slide var(--dur) var(--ease);
  }
  @keyframes slide {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }
  .head {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 4) calc(var(--space) * 5);
    border-bottom: 1px solid var(--line-soft);
  }
  .head .small {
    flex: 1;
  }
  .close {
    width: 32px;
    height: 32px;
    border: 1px solid var(--line-ctl);
    border-radius: var(--radius-ctl);
    background: var(--fill-2);
    color: var(--text-2);
    cursor: pointer;
  }
  .close:hover {
    color: var(--text);
    background: var(--fill-3);
  }
  .lines {
    flex: 1;
    overflow-y: auto;
    padding: calc(var(--space) * 3) calc(var(--space) * 5);
    font-size: var(--fs-small);
  }
  .line {
    display: flex;
    gap: calc(var(--space) * 2);
    padding: 1px 0;
  }
  .mark {
    width: 12px;
    text-align: center;
  }
  .info .mark {
    color: var(--ok);
  }
  .warn .mark {
    color: var(--warn);
  }
  .error .mark {
    color: var(--bad);
  }
</style>
