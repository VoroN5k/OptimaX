<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { JournalRecord } from "../lib/bindings/JournalRecord";
  import { dateTime, size } from "../lib/format";
  import { tk } from "../lib/i18n";
  import { logLocal } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import StateView from "./StateView.svelte";

  // Журнал очищення (локальний файл): що, коли, за яким правилом, скільки звільнено.
  let { onchange }: { onchange: () => void } = $props();

  /** Рядків одного запису в DOM — не більше (CLAUDE.md, 10.3). */
  const LINES_IN_DOM = 300;

  let records = $state<JournalRecord[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let busy = $state(false);

  onMount(async () => {
    try {
      records = (await api.cleanupJournal()).slice().reverse();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  });

  async function hibernateOn() {
    busy = true;
    error = null;
    try {
      await api.cleanupHibernate(true, false);
      logLocal("info", tk("ui.clean.hibernate_on_done"));
      onchange();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      busy = false;
    }
  }

  const outcome = (o: JournalRecord["lines"][number]["outcome"]) =>
    o.kind === "skipped" ? `${tk("ui.clean.outcome.skipped")}: ${o.reason}` : tk(`ui.clean.outcome.${o.kind}`);
</script>

{#if loading}
  <Card><StateView kind="loading" title={tk("ui.common.loading")} /></Card>
{:else if error && !records.length}
  <Card><StateView kind="error" title={tk("ui.common.error")} text={error} /></Card>
{:else if !records.length}
  <Card><StateView kind="empty" title={tk("ui.clean.j_empty_title")} text={tk("ui.clean.j_empty_text")} /></Card>
{:else}
  {#if error}<p class="err small" role="alert">{error}</p>{/if}
  <div class="list">
    {#each records as r (r.id)}
      <Card>
        <div class="head">
          <div>
            <h3>{tk(`ui.clean.run.${r.kind}`)} <span class="dim small">#{r.id}</span></h3>
            <p class="small muted">
              {dateTime(r.started_unix)}
              {#if r.finished_unix == null}· <span class="warn">{tk("ui.clean.j_unfinished")}</span>{/if}
            </p>
          </div>
          <div class="nums small">
            {#if r.freed_bytes}<span>{tk("ui.clean.j_freed", { size: size(r.freed_bytes) })}</span>{/if}
            {#if r.quarantined_bytes}<span>{tk("ui.clean.j_quarantined", { size: size(r.quarantined_bytes) })}</span>{/if}
            {#if r.restored_bytes}<span>{tk("ui.clean.j_restored", { size: size(r.restored_bytes) })}</span>{/if}
            {#if r.skipped}<span class="warn">{tk("ui.clean.j_skipped", { n: r.skipped })}</span>{/if}
          </div>
          {#if r.kind === "hibernate_off" && r.finished_unix != null}
            <Button small variant="secondary" icon="undo" disabled={busy} onclick={hibernateOn}>{tk("ui.clean.hibernate_on")}</Button>
          {/if}
        </div>
        {#if r.note}<pre class="note mono">{r.note}</pre>{/if}
        {#if r.lines.length}
          <details>
            <summary class="small muted">
              {tk("ui.clean.j_lines", { n: r.lines_total })}{#if r.lines_total > r.lines.length}
                · {tk("ui.clean.j_lines_cut", { n: r.lines.length })}{/if}
            </summary>
            <div class="lines mono">
              {#each r.lines.slice(0, LINES_IN_DOM) as l, i (i)}
                <div class="line" class:skip={l.outcome.kind === "skipped"}>
                  <span class="o">{outcome(l.outcome)}</span>
                  <span class="dim">{size(l.bytes)}</span>
                  <span class="p" title={l.path}>{l.path}</span>
                  {#if l.rule_id}<span class="dim">[{l.rule_id}]</span>{/if}
                </div>
              {/each}
            </div>
          </details>
        {/if}
      </Card>
    {/each}
  </div>
{/if}

<style>
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: calc(var(--space) * 4);
  }
  .nums {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 3);
    margin-left: auto;
  }
  p {
    margin: 0;
  }
  .note {
    margin: calc(var(--space) * 3) 0 0;
    padding: calc(var(--space) * 2) calc(var(--space) * 3);
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    color: var(--text-muted);
    font-size: 11.5px;
  }
  details {
    margin-top: calc(var(--space) * 3);
  }
  summary {
    cursor: pointer;
  }
  .lines {
    max-height: 260px;
    overflow: auto;
    margin-top: calc(var(--space) * 2);
    font-size: 11.5px;
  }
  .line {
    display: flex;
    gap: calc(var(--space) * 2);
    white-space: nowrap;
  }
  .line .p {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .o {
    color: var(--ok);
    flex-shrink: 0;
  }
  .skip .o,
  .warn {
    color: var(--warn);
  }
  .err {
    color: var(--danger);
  }
</style>
