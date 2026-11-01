<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { CmdError } from "../lib/bindings/CmdError";
  import type { LagCapture } from "../lib/bindings/LagCapture";
  import type { LagView } from "../lib/bindings/LagView";
  import type { WatchStatus } from "../lib/bindings/WatchStatus";
  import { t, tk } from "../lib/i18n";
  import { flashBg, logLocal, ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import FindingCard from "./FindingCard.svelte";
  import FrametimeChart from "./FrametimeChart.svelte";
  import StateView from "./StateView.svelte";

  // «Щойно лагнуло»: стеження з кільцевим буфером. Буфер, знімки й аналіз — на бекенді;
  // тут лише кнопки, стан і показ знімків.
  // `heading` — заголовок картки (на окремій вкладці його дає сторінка).
  let { game, heading = true }: { game: string; heading?: boolean } = $props();

  let view = $state<LagView | null>(null);
  let status = $state<WatchStatus | null>(null);
  let busy = $state(false);
  let error = $state<CmdError | null>(null);
  let opened = $state<number | null>(null);

  const watching = $derived(view?.watching ?? false);
  const windowS = $derived(view?.window_s ?? 30);

  // Фонова анімація вимикається, поки йде стеження (як під час запису).
  let wasWatching = false;
  $effect(() => {
    if (watching) ui.measuring = true;
    else if (wasWatching) ui.measuring = false;
    wasWatching = watching;
  });

  function apply(v: LagView | null) {
    const before = view?.captures[0]?.id;
    view = v;
    status = v?.status ?? null;
    const newest = v?.captures[0];
    if (newest && newest.id !== before) {
      opened = newest.id;
      flashBg(newest.result.diagnosis.verdicts.some((x) => x.finding.impact !== "info") ? "warn" : "ok");
      logLocal("info", tk("ui.lag.done_log", { game: v.game }));
    }
  }

  onMount(() => {
    void api
      .lagState()
      .then((v) => {
        view = v;
        status = v?.status ?? null;
        opened = v?.captures[0]?.id ?? null;
      })
      .catch(() => undefined);
    const subs = [on("lag-status", (s) => (status = s)), on("lag-update", (v) => apply(v))];
    return () => subs.forEach((p) => void p.then((u) => u()));
  });

  async function run(f: () => Promise<void>) {
    error = null;
    busy = true;
    try {
      await f();
    } catch (e) {
      error = asCmdError(e);
    } finally {
      busy = false;
    }
  }

  const start = () =>
    run(async () => {
      view = await api.lagStart(game.trim());
      status = null;
      opened = null;
    });
  const stop = () => run(async () => apply(await api.lagStop()));
  const capture = () => run(() => api.lagCapture());
  const close = () =>
    run(async () => {
      await api.lagClose();
      view = null;
      status = null;
    });

  function statusText(s: WatchStatus | null): string {
    if (!s) return tk("ui.lag.starting");
    const secs = Math.round(s.buffered_s);
    if (!s.game_running) return tk("ui.lag.waiting", { game: view?.game ?? "" });
    return s.in_game ? tk("ui.lag.in_game", { s: secs }) : tk("ui.lag.paused", { s: secs });
  }

  function endText(v: LagView): string | null {
    if (!v.ended) return null;
    if (typeof v.ended === "object") return tk("ui.lag.end.present_mon_failed", { msg: v.ended.present_mon_failed });
    return tk(`ui.lag.end.${v.ended}`);
  }

  const time = (c: LagCapture) =>
    c.at_unix_ms == null ? `#${c.id}` : new Date(c.at_unix_ms).toLocaleTimeString("uk-UA");
  const fps = (v: number | undefined) => (v == null ? "—" : v.toFixed(1));
</script>

<Card title={heading ? tk("ui.lag.title") : undefined} subtitle={heading ? tk("ui.lag.subtitle") : undefined}>
  {#if !view || (!watching && view.captures.length === 0 && !view.ended)}
    <p class="small muted">{tk("ui.lag.how", { s: windowS })}</p>
  {:else if watching}
    <p class="status" role="status" aria-live="polite">
      <span class="dot" class:live={status?.in_game}></span>
      {statusText(status)}
    </p>
    {#if status && !status.dpc}
      <p class="small dim">{tk("ui.lag.no_dpc")}</p>
    {/if}
  {:else if endText(view)}
    <p class="small muted" role="status">{endText(view)}</p>
  {/if}

  {#if view?.last_failure}
    <p class="warn small" role="alert">{tk(`ui.lag.fail.${view.last_failure}`, { s: windowS })}</p>
  {/if}
  {#if error}
    <StateView kind="error" title={tk("ui.common.error")} text={error.message} />
  {/if}

  {#if view && (watching || view.captures.length)}
    <h4 class="small muted">{tk("ui.lag.captures")}</h4>
    {#if view.captures.length === 0}
      <p class="small dim">{tk("ui.lag.empty")}</p>
    {:else}
      <ul class="captures">
        {#each view.captures as c (c.id)}
          {@const d = c.result.diagnosis}
          <li>
            <div class="row">
              <span class="mono">{time(c)}</span>
              <span class="title">{d.verdicts[0] ? t(d.verdicts[0].finding.title) : ""}</span>
              <span class="mono muted">
                {tk("ui.lag.item", {
                  avg: fps(d.stats?.avg_fps),
                  low: fps(d.stats?.low_1pct_fps),
                  s: Math.round(c.result.counted_s),
                })}
              </span>
              <Button small variant="secondary" onclick={() => (opened = opened === c.id ? null : c.id)}>
                {opened === c.id ? tk("ui.lag.hide") : tk("ui.lag.open")}
              </Button>
            </div>
            {#if opened === c.id}
              <div class="detail">
                {#each d.verdicts.slice(1) as v, i (`${c.id}-${i}`)}
                  <FindingCard finding={v.finding} collapsible expanded={i === 0} />
                {:else}
                  <p class="small muted">{tk("ui.diag.no_more")}</p>
                {/each}
                {#if d.chart}
                  <FrametimeChart chart={d.chart} />
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  {/if}

  <div class="actions">
    {#if watching}
      <Button variant="secondary" icon="stop" disabled={busy} onclick={stop}>{tk("ui.lag.stop")}</Button>
      <Button variant="primary" disabled={busy} onclick={capture}>{tk("ui.lag.capture")}</Button>
    {:else}
      {#if view}
        <Button variant="secondary" disabled={busy} onclick={close}>{tk("ui.lag.close")}</Button>
      {/if}
      {#if game.trim()}
        <Button variant="primary" icon="play" loading={busy} disabled={busy} onclick={start}>
          {tk("ui.lag.start", { game: game.trim() })}
        </Button>
      {:else}
        <span class="small dim">{tk("ui.lag.pick_game")}</span>
      {/if}
    {/if}
  </div>
</Card>

<style>
  .status {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    margin: 0;
    font-size: 13px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .dot.live {
    background: var(--ok);
  }
  .warn {
    margin: calc(var(--space) * 3) 0 0;
    color: var(--warn);
  }
  h4 {
    margin: calc(var(--space) * 5) 0 calc(var(--space) * 2);
    font-weight: 600;
  }
  .captures {
    display: grid;
    gap: calc(var(--space) * 2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .captures li {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
  }
  .row {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 2) calc(var(--space) * 3);
    font-size: 13px;
  }
  .row .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .detail {
    display: grid;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 3);
    border-top: 1px solid var(--border);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: calc(var(--space) * 2);
    margin-top: calc(var(--space) * 4);
  }
</style>
