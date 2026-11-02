<script lang="ts">
  import { onMount } from "svelte";
  import { api, on } from "../lib/api";
  import type { BlackBoxView } from "../lib/bindings/BlackBoxView";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";

  // Чорна скринька на «Головній»: сесія, що йде, або підсумок останньої (за 3 дні) —
  // скільки фризів і з якими причинами. Причини рахує бекенд.

  /** Скільки показувати підсумок після кінця сесії. */
  const SHOW_FOR_MS = 3 * 86_400_000;

  let view = $state<BlackBoxView | null>(null);

  onMount(() => {
    void api
      .blackboxState()
      .then((v) => (view = v))
      .catch(() => (view = null));
    const off = on("blackbox", (v) => (view = v));
    return () => void off.then((f) => f());
  });

  function played(s: number): string {
    const m = Math.round(s / 60);
    return m >= 60 ? tk("ui.blackbox.hours", { h: Math.floor(m / 60), m: m % 60 }) : tk("ui.blackbox.minutes", { m });
  }

  function causeName(code: string): string {
    return code === "unknown" ? tk("ui.blackbox.cause_unknown") : tk(`diag.cause_name.${code}`);
  }

  const last = $derived(
    view?.last &&
      !view.active &&
      ui.blackboxHidden !== view.last.session.id &&
      Date.now() - view.last.session.ended_unix_ms < SHOW_FOR_MS
      ? view.last
      : null,
  );
</script>

{#if view?.active}
  {@const a = view.active}
  <div class="box live" role="status">
    <i class="dot" aria-hidden="true"></i>
    <span class="small">
      {a.paused
        ? tk("ui.blackbox.paused", { game: a.game })
        : tk("ui.blackbox.active", { game: a.game, played: played(a.played_s), n: a.freezes })}
    </span>
  </div>
{:else if last}
  {@const s = last.session}
  <div class="box" role="status">
    <i class="dot" class:warn={s.freezes.length > 0} aria-hidden="true"></i>
    <div class="text">
      <strong>
        {s.freezes.length
          ? tk("ui.blackbox.last", { game: s.game, played: played(s.played_s), n: s.freezes.length, ms: s.threshold_ms })
          : tk("ui.blackbox.last_none", { game: s.game, played: played(s.played_s), ms: s.threshold_ms })}
      </strong>
      {#if last.causes.length}
        <span class="causes">
          {#each last.causes as c (c.cause)}
            <span class="chip small">{causeName(c.cause)} · {c.count}</span>
          {/each}
        </span>
        <span class="small dim">{tk("ui.blackbox.note")}</span>
      {/if}
    </div>
    <Button small variant="ghost" onclick={() => (ui.blackboxHidden = s.id)}>{tk("ui.blackbox.close")}</Button>
  </div>
{/if}

<style>
  .box {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    margin-bottom: calc(var(--space) * 4);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-card);
  }
  .live {
    border-color: var(--border-strong);
  }
  .dot {
    width: 8px;
    height: 8px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--ok);
  }
  .live .dot {
    background: var(--danger);
  }
  .dot.warn {
    background: var(--warn);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: calc(var(--space) * 1);
  }
  .causes {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 2);
  }
  .chip {
    padding: 1px calc(var(--space) * 2);
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    color: var(--text);
  }
</style>
