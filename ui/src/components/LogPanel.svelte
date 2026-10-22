<script lang="ts">
  import { tick } from "svelte";
  import { log } from "../lib/state.svelte";
  import { time } from "../lib/format";
  import { tk } from "../lib/i18n";

  // Журнал дій: моноширинний, автопрокрутка, у DOM не більше 500 рядків (обрізання в state).
  let open = $state(false);
  let box = $state<HTMLDivElement | undefined>();

  $effect(() => {
    const n = log.lines.length;
    if (open && box && n) {
      void tick().then(() => {
        if (box) box.scrollTop = box.scrollHeight;
      });
    }
  });
</script>

<section class="log" class:open aria-label={tk("ui.log.title")}>
  <button type="button" class="head" aria-expanded={open} onclick={() => (open = !open)}>
    <span class="dot" class:has={log.lines.length > 0} aria-hidden="true"></span>
    {tk("ui.log.title")}
    <span class="muted small">{tk("ui.log.count", { n: log.lines.length })}</span>
  </button>
  {#if open}
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
  {/if}
</section>

<style>
  .log {
    border-top: 1px solid var(--border);
    background: var(--bg-sidebar);
  }
  .head {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    width: 100%;
    padding: calc(var(--space) * 2) calc(var(--space) * 6);
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 12px;
    cursor: pointer;
    text-align: left;
  }
  .head:hover {
    color: var(--text);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .dot.has {
    background: var(--ok);
  }
  .lines {
    max-height: 180px;
    overflow-y: auto;
    padding: 0 calc(var(--space) * 6) calc(var(--space) * 3);
    font-size: 12px;
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
    color: var(--danger);
  }
</style>
