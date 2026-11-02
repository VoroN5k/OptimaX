<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { DiagnosisResult } from "../lib/bindings/DiagnosisResult";
  import type { HistoryEntry } from "../lib/bindings/HistoryEntry";
  import type { HistoryView } from "../lib/bindings/HistoryView";
  import type { RecordComparison } from "../lib/bindings/RecordComparison";
  import type { StateChange } from "../lib/bindings/StateChange";
  import { dateTime } from "../lib/format";
  import { t, tk } from "../lib/i18n";
  import { logLocal, ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import Modal from "./Modal.svelte";
  import ResultView from "./ResultView.svelte";
  import Segmented from "./Segmented.svelte";
  import StateView from "./StateView.svelte";

  // Історія записів «Чому лагає?»: записи гри в часі, що змінилося між ними, порівняння двох
  // записів і повторний аналіз старого запису. Порівняння й зміни рахує бекенд.

  let view = $state<HistoryView | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let game = $state<string>("");
  let selected = $state<string[]>([]);
  let comparison = $state<RecordComparison | null>(null);
  let opened = $state<{ entry: HistoryEntry; result: DiagnosisResult } | null>(null);
  let busy = $state(false);
  let confirmDelete = $state<HistoryEntry | null>(null);
  let confirmClear = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      view = await api.historyList();
      const games = gamesOf(view);
      if (!games.includes(game)) game = games[0] ?? "";
      selected = selected.filter((id) => view?.rows.some((r) => r.entry.id === id));
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load().then(() => {
      // З «Головної»: вибрати пару записів і одразу порівняти.
      const pair = ui.pendingCompare;
      ui.pendingCompare = null;
      const first = pair && view?.rows.find((r) => r.entry.id === pair[0]);
      if (pair && first) {
        game = first.entry.game;
        selected = [...pair];
        void compareSelected();
      }
    });
  });

  function gamesOf(v: HistoryView | null): string[] {
    const out: string[] = [];
    for (const r of v?.rows ?? []) if (!out.includes(r.entry.game)) out.push(r.entry.game);
    return out;
  }

  const games = $derived(gamesOf(view));
  const rows = $derived((view?.rows ?? []).filter((r) => r.entry.game === game));

  function toggle(id: string) {
    comparison = null;
    if (selected.includes(id)) selected = selected.filter((x) => x !== id);
    else selected = [...selected.slice(-1), id];
  }

  async function run(f: () => Promise<void>) {
    busy = true;
    error = null;
    try {
      await f();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      busy = false;
    }
  }

  const compareSelected = () =>
    run(async () => {
      const [a, b] = selected;
      if (a && b) {
        opened = null;
        comparison = await api.historyCompare(a, b);
      }
    });

  const open = (entry: HistoryEntry) =>
    run(async () => {
      comparison = null;
      opened = { entry, result: await api.historyOpen(entry.id) };
    });

  const remove = (entry: HistoryEntry) =>
    run(async () => {
      await api.historyDelete(entry.id);
      if (opened?.entry.id === entry.id) opened = null;
      comparison = null;
      selected = selected.filter((x) => x !== entry.id);
      confirmDelete = null;
      logLocal("info", tk("ui.history.deleted_log", { game: entry.game }));
      await load();
    });

  const clearAll = () =>
    run(async () => {
      const n = await api.historyClear();
      confirmClear = false;
      opened = null;
      comparison = null;
      selected = [];
      logLocal("info", tk("ui.history.cleared_log", { n }));
      await load();
    });

  const fps = (v: number | null) => (v == null ? "—" : v.toFixed(0));
  const pct = (v: number | null) => (v == null ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(1)} %`);
  const when = (ms: number) => dateTime(Math.floor(ms / 1000));

  /** Стани HAGS / ігрового режиму приходять назвами (`enabled`…) — показуємо словами. */
  const STATES = new Set(["enabled", "disabled", "not_set", "default"]);
  function value(c: StateChange, v: string): string {
    if ((c.field === "hags" || c.field === "game_mode") && STATES.has(v)) return tk(`ui.history.state.${v}`);
    return v;
  }
</script>

<div class="history">
  {#if loading && !view}
    <Card><StateView kind="loading" title={tk("ui.history.loading")} /></Card>
  {:else if error && !view}
    <Card>
      <StateView kind="error" title={tk("ui.common.error")} text={error}>
        {#snippet action()}<Button onclick={load}>{tk("ui.common.retry")}</Button>{/snippet}
      </StateView>
    </Card>
  {:else if view && view.rows.length === 0}
    <Card><StateView kind="empty" title={tk("ui.history.empty_title")} text={tk("ui.history.empty_text")} /></Card>
  {:else if view}
    <Card title={tk("ui.history.title")} subtitle={tk("ui.history.scene_note")}>
      {#if games.length > 1}
        <div class="games">
          <Segmented
            label={tk("ui.history.game")}
            bind:value={game}
            onchange={() => {
              selected = [];
              comparison = null;
            }}
            options={games.map((g) => ({ value: g, label: g }))}
          />
        </div>
      {/if}
      <ul class="rows">
        {#each rows as r (r.entry.id)}
          {@const e = r.entry}
          <li class:picked={selected.includes(e.id)}>
            <label class="pick">
              <input
                type="checkbox"
                checked={selected.includes(e.id)}
                onchange={() => toggle(e.id)}
                aria-label={tk("ui.history.pick", { when: when(e.at_unix_ms) })}
              />
            </label>
            <div class="main">
              <div class="line">
                <span class="mono when">{when(e.at_unix_ms)}</span>
                <span class="title">{t(e.title)}</span>
                {#if e.scene}<span class="scene small">{e.scene}</span>{/if}
                <span class="mono muted">{tk("ui.history.fps", { avg: fps(e.avg_fps), low: fps(e.low_1pct_fps) })}</span>
              </div>
              {#if r.changes.length}
                <div class="chips">
                  <span class="small dim">{tk("ui.history.changed")}</span>
                  {#each r.changes as c (c.field)}
                    <span class="chip small" title={`${value(c, c.from)} → ${value(c, c.to)}`}>
                      {tk(`ui.history.field.${c.field}`)}
                    </span>
                  {/each}
                </div>
              {/if}
            </div>
            <div class="actions">
              {#if e.has_recording}
                <Button small variant="secondary" disabled={busy} onclick={() => open(e)}>{tk("ui.history.open")}</Button>
              {/if}
              <Button
                small
                variant="ghost"
                icon="trash"
                disabled={busy}
                label={tk("ui.history.delete")}
                onclick={() => (confirmDelete = e)}
              ></Button>
            </div>
          </li>
        {/each}
      </ul>
      <div class="foot">
        <Button small variant="ghost" disabled={busy} onclick={() => (confirmClear = true)}>{tk("ui.history.clear")}</Button>
        <!-- Порожній span лишається, щоб кнопки не зсувались. -->
        <span class="small muted">{selected.length < 2 ? tk("ui.history.pick_hint") : ""}</span>
        <Button small variant="primary" disabled={busy || selected.length !== 2} onclick={compareSelected}>
          {tk("ui.history.compare")}
        </Button>
      </div>
      {#if view.broken > 0}
        <p class="small dim">{tk("ui.history.broken", { n: view.broken })}</p>
      {/if}
      {#if error}
        <p class="small err" role="alert">{error}</p>
      {/if}
    </Card>

    {#if comparison}
      {@const c = comparison}
      <Card
        title={tk("ui.history.cmp_title")}
        subtitle={tk("ui.history.cmp_between", { from: when(c.older.at_unix_ms), to: when(c.newer.at_unix_ms) })}
      >
        <div class="cmp">
          <div class="delta {c.trend}">
            <span class="small muted">{tk("ui.diag.avg")}</span>
            <strong class="mono">{fps(c.older.avg_fps)} → {fps(c.newer.avg_fps)}</strong>
            <span class="mono">{pct(c.avg_fps_change_pct)}</span>
          </div>
          <div class="delta">
            <span class="small muted">1% low</span>
            <strong class="mono">{fps(c.older.low_1pct_fps)} → {fps(c.newer.low_1pct_fps)}</strong>
            <span class="mono">{pct(c.low_1pct_change_pct)}</span>
          </div>
          <p class="trend {c.trend}" role="status">
            <i class="dot" aria-hidden="true"></i>
            {tk(`ui.history.trend.${c.trend}`, { t: c.threshold_pct.toFixed(0) })}
          </p>
        </div>
        {#if !c.same_game}
          <p class="warn small">{tk("ui.history.warn_game")}</p>
        {/if}
        {#if c.scene === "different"}
          <p class="warn small">{tk("ui.history.warn_scene", { a: c.older.scene ?? "", b: c.newer.scene ?? "" })}</p>
        {:else if c.scene === "unknown"}
          <p class="small muted note">{tk("ui.history.scene_unknown")}</p>
        {/if}
        {#if !c.same_display}
          <p class="warn small">{tk("ui.history.warn_display")}</p>
        {/if}
        {#if c.bottleneck_changed}
          <p class="small">{tk("ui.history.bottleneck_changed", { from: t(c.older.title), to: t(c.newer.title) })}</p>
        {/if}
        <h3 class="section muted">{tk("ui.history.what_changed")}</h3>
        {#if c.changes.length}
          <dl class="changes">
            {#each c.changes as ch (ch.field)}
              <dt>{tk(`ui.history.field.${ch.field}`)}</dt>
              <dd class="mono">{value(ch, ch.from)} → {value(ch, ch.to)}</dd>
            {/each}
          </dl>
        {:else}
          <p class="small muted">{tk("ui.history.no_changes")}</p>
        {/if}
      </Card>
    {/if}

    {#if opened}
      <div class="opened" role="note">
        <span>{tk("ui.history.opened", { when: when(opened.entry.at_unix_ms) })}</span>
        <Button small variant="ghost" onclick={() => (opened = null)}>{tk("ui.history.close")}</Button>
      </div>
      <ResultView result={opened.result} />
    {/if}
  {/if}
</div>

{#if confirmDelete}
  {@const e = confirmDelete}
  <Modal
    title={tk("ui.history.delete_title")}
    confirmLabel={tk("ui.history.delete")}
    danger
    {busy}
    onconfirm={() => remove(e)}
    oncancel={() => (confirmDelete = null)}
  >
    <p>{tk("ui.history.delete_text", { game: e.game, when: when(e.at_unix_ms) })}</p>
  </Modal>
{/if}

{#if confirmClear}
  <Modal
    title={tk("ui.history.clear_title")}
    confirmLabel={tk("ui.history.clear")}
    danger
    {busy}
    onconfirm={clearAll}
    oncancel={() => (confirmClear = false)}
  >
    <p>{tk("ui.history.clear_text", { n: view?.rows.length ?? 0 })}</p>
  </Modal>
{/if}

<style>
  .history {
    display: grid;
    gap: calc(var(--space) * 4);
  }
  .games {
    margin-bottom: calc(var(--space) * 3);
  }
  .rows {
    display: grid;
    gap: calc(var(--space) * 2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .rows li {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 2) calc(var(--space) * 3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    transition: border-color var(--dur) var(--ease);
  }
  .rows li.picked {
    border-color: var(--accent-from);
  }
  .pick input {
    width: 16px;
    height: 16px;
    accent-color: var(--accent-from);
    cursor: pointer;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: calc(var(--space) * 1);
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: calc(var(--space) * 3);
    font-size: 13px;
  }
  .when {
    flex-shrink: 0;
    color: var(--text-muted);
    font-size: 12px;
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .scene {
    flex-shrink: 0;
    padding: 1px calc(var(--space) * 2);
    border-radius: 999px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .note {
    margin: calc(var(--space) * 3) 0 0;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 1) calc(var(--space) * 2);
  }
  .chip {
    padding: 1px calc(var(--space) * 2);
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    color: var(--warn);
  }
  .actions {
    display: flex;
    gap: calc(var(--space) * 1);
    flex-shrink: 0;
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
    margin-top: calc(var(--space) * 4);
  }
  .err {
    color: var(--danger);
  }
  .cmp {
    display: flex;
    flex-wrap: wrap;
    align-items: stretch;
    gap: calc(var(--space) * 3);
  }
  .delta {
    display: grid;
    gap: 2px;
    min-width: 180px;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  .delta strong {
    font-size: 20px;
    font-weight: 600;
  }
  .trend {
    flex: 1;
    min-width: 220px;
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    margin: 0;
    font-size: 14px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
  }
  .trend.better .dot {
    background: var(--ok);
  }
  .trend.worse .dot {
    background: var(--warn);
  }
  .warn {
    margin: calc(var(--space) * 3) 0 0;
    color: var(--warn);
  }
  .section {
    margin: calc(var(--space) * 4) 0 calc(var(--space) * 2);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .changes {
    display: grid;
    grid-template-columns: minmax(160px, max-content) 1fr;
    gap: 4px calc(var(--space) * 4);
    margin: 0;
    padding: calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-size: 13px;
  }
  .changes dt {
    color: var(--text-muted);
  }
  .changes dd {
    margin: 0;
    word-break: break-word;
  }
  .opened {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 2) calc(var(--space) * 4);
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-md);
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
