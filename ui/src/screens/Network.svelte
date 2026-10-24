<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { Adapter } from "../lib/bindings/Adapter";
  import type { NetProgress } from "../lib/bindings/NetProgress";
  import type { NetResult } from "../lib/bindings/NetResult";
  import type { NetView } from "../lib/bindings/NetView";
  import type { PingStats } from "../lib/bindings/PingStats";
  import { tk } from "../lib/i18n";
  import { flashBg, logLocal } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import FindingCard from "../components/FindingCard.svelte";
  import Page from "../components/Page.svelte";
  import Segmented from "../components/Segmented.svelte";
  import StateView from "../components/StateView.svelte";

  let view = $state<NetView | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let remote = $state("");
  let seconds = $state(30);
  let testing = $state(false);
  let progress = $state<NetProgress | null>(null);
  let result = $state<NetResult | null>(null);

  async function load() {
    loading = true;
    error = null;
    try {
      view = await api.netInfo();
      if (!remote && view.default_remote) remote = view.default_remote;
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
    const subs = [
      on("net-progress", (p) => (progress = p)),
      on("net-done", (r) => {
        result = r;
        testing = false;
        const actionable = r.findings.some((f) => f.impact !== "info");
        flashBg(actionable ? "warn" : "ok");
        logLocal("info", tk("ui.net.done_log"));
      }),
      on("net-error", (e) => {
        error = e.message;
        testing = false;
      }),
    ];
    return () => subs.forEach((p) => void p.then((u) => u()));
  });

  async function start() {
    error = null;
    result = null;
    progress = null;
    try {
      await api.netTestStart(remote.trim() || null, seconds);
      testing = true;
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  const link = $derived(view?.info.adapters.find((a) => a.carries_internet) ?? null);
  const others = $derived(view?.info.adapters.filter((a) => !a.carries_internet) ?? []);
  const share = $derived(progress ? Math.min(1, progress.elapsed_s / progress.total_s) : 0);
  const kindLabel = (a: Adapter) => tk(`ui.net.kind.${a.kind}`);
  const num = (v: number | null | undefined, d = 1) => (v == null ? "—" : v.toFixed(d));
  const ms = (v: number | null | undefined) => tk("ui.net.ms", { v: num(v, 2) });
</script>

{#snippet statsRow(label: string, s: PingStats | null | undefined)}
  <div class="srow">
    <span class="slabel">{label}<span class="target dim mono small">{s?.target ?? ""}</span></span>
    {#if s}
      <span class="mono">{ms(s.avg_ms)}</span>
      <span class="mono">{ms(s.p95_ms)}</span>
      <span class="mono">{ms(s.jitter_ms)}</span>
      <span class="mono">{num(s.loss_pct)}%</span>
      <span class="mono dim">{s.received}/{s.sent}</span>
    {:else}
      <span class="dim">—</span><span></span><span></span><span></span><span></span>
    {/if}
  </div>
{/snippet}

<Page title={tk("ui.net.title")} subtitle={tk("ui.net.subtitle")}>
  {#if loading}
    <Card><StateView kind="loading" title={tk("ui.net.loading")} /></Card>
  {:else if !view}
    <Card><StateView kind="error" title={tk("ui.common.error")} text={error ?? ""} /></Card>
  {:else}
    <Card title={tk("ui.net.link")}>
      {#if link}
        <div class="link">
          <div>
            <h2>{kindLabel(link)} · {link.name}</h2>
            <p class="small muted">{link.description}</p>
          </div>
          <dl class="kv">
            <dt>{tk("ui.net.speed")}</dt><dd class="mono">{link.link_mbps != null ? tk("ui.net.mbps", { v: link.link_mbps }) : "—"}</dd>
            <dt>{tk("ui.net.gateway")}</dt><dd class="mono">{link.gateway ?? "—"}</dd>
            <dt>{tk("ui.net.dns")}</dt><dd class="mono">{link.dns.join(", ") || "—"}</dd>
          </dl>
        </div>
        {#if link.wifi}
          {@const w = link.wifi}
          <dl class="kv wifi">
            <dt>{tk("ui.net.ssid")}</dt><dd>{w.ssid ?? "—"}</dd>
            <dt>{tk("ui.net.band")}</dt><dd>{w.band ? tk(`ui.net.band.${w.band}`) : "—"}{#if w.channel} · {tk("ui.net.channel", { v: w.channel })}{/if}</dd>
            <dt>{tk("ui.net.signal")}</dt><dd class="mono">{w.signal_pct != null ? `${w.signal_pct}%` : "—"}{#if w.rssi_dbm != null} · {tk("ui.net.dbm", { v: w.rssi_dbm })}{/if}</dd>
            <dt>{tk("ui.net.standard")}</dt><dd>{w.standard ?? "—"}</dd>
            <dt>{tk("ui.net.rate")}</dt><dd class="mono">{w.rx_mbps} / {w.tx_mbps} {tk("ui.net.mbps_unit")}</dd>
          </dl>
          {#if w.details_blocked}<p class="small warn">{tk("ui.net.location_hint")}</p>{/if}
        {/if}
      {:else}
        <StateView kind="empty" title={tk("ui.net.no_link_title")} text={tk("ui.net.no_link_text")} />
      {/if}
      {#if others.length}
        <p class="small muted others">
          {tk("ui.net.others")}
          {#each others as a, i (a.name)}{i ? ", " : " "}{a.name} ({kindLabel(a)}){/each}
        </p>
      {/if}
    </Card>

    {#if view.findings.length && !result}
      <div class="list">
        {#each view.findings as f (f.id)}<FindingCard finding={f} />{/each}
      </div>
    {/if}

    <Card title={tk("ui.net.test")} subtitle={tk("ui.net.test_hint")}>
      <div class="setup">
        <div class="field">
          <label class="small muted" for="remote">{tk("ui.net.remote")}</label>
          <input id="remote" class="mono" bind:value={remote} placeholder={tk("ui.net.remote_placeholder")} disabled={testing} />
          <p class="small dim">{tk(view.default_remote ? "ui.net.remote_hint_dns" : "ui.net.remote_hint_empty")}</p>
        </div>
        <div class="field">
          <span class="small muted">{tk("ui.net.duration")}</span>
          <Segmented
            label={tk("ui.net.duration")}
            bind:value={seconds}
            options={[
              { value: 15, label: "15 с" },
              { value: 30, label: "30 с" },
              { value: 60, label: "60 с" },
            ]}
          />
        </div>
      </div>

      {#if testing || result}
        {@const g = result ? result.measurement.gateway : progress?.gateway}
        {@const r = result ? result.measurement.remote : progress?.remote}
        {#if testing}
          <div class="track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(share * 100)}>
            <div class="fill" style:transform="scaleX({share})"></div>
          </div>
        {/if}
        <div class="table" aria-live="polite">
          <div class="srow head small muted">
            <span></span><span>{tk("ui.net.col.avg")}</span><span>{tk("ui.net.col.p95")}</span>
            <span>{tk("ui.net.col.jitter")}</span><span>{tk("ui.net.col.loss")}</span><span>{tk("ui.net.col.replies")}</span>
          </div>
          {@render statsRow(tk("ui.net.router"), g)}
          {#if remote.trim() || r}{@render statsRow(tk("ui.net.remote_row"), r)}{/if}
        </div>
      {/if}
      {#if error}<p class="small warn">{error}</p>{/if}
    </Card>

    {#if result}
      <div class="list">
        {#each result.findings as f (f.id)}<FindingCard finding={f} />{/each}
      </div>
      {#if result.cancelled}<p class="small dim">{tk("ui.net.cancelled")}</p>{/if}
    {/if}

    <Card title={tk("ui.net.no_tweaks_title")}>
      <p class="muted">{tk("ui.net.no_tweaks_text")}</p>
    </Card>
  {/if}

  {#snippet secondary()}
    {#if testing}
      <Button variant="secondary" icon="stop" onclick={() => api.netTestCancel()}>{tk("ui.net.stop")}</Button>
    {:else}
      <Button variant="ghost" icon="undo" onclick={load}>{tk("ui.net.refresh")}</Button>
    {/if}
  {/snippet}
  {#snippet primary()}
    <Button variant="primary" icon="play" disabled={testing || !view} loading={testing} onclick={start}>
      {tk("ui.net.start")}
    </Button>
  {/snippet}
</Page>

<style>
  .link {
    display: flex;
    justify-content: space-between;
    gap: calc(var(--space) * 6);
    align-items: flex-start;
  }
  .link h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .link p {
    margin: calc(var(--space) * 1) 0 0;
  }
  .kv {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 1.5) calc(var(--space) * 5);
    margin: 0;
  }
  .kv.wifi {
    margin-top: calc(var(--space) * 4);
    padding-top: calc(var(--space) * 4);
    border-top: 1px solid var(--border);
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
  }
  .others {
    margin: calc(var(--space) * 4) 0 0;
  }
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .setup {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: calc(var(--space) * 6);
    align-items: start;
  }
  .field {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .field p {
    margin: 0;
  }
  input {
    padding: calc(var(--space) * 2.5) calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text);
  }
  input:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 1px;
  }
  .track {
    height: 6px;
    margin-top: calc(var(--space) * 5);
    border-radius: 999px;
    background: var(--bg-input);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    transform-origin: left;
    background: var(--accent-gradient);
    transition: transform var(--dur-slow) var(--ease);
  }
  .table {
    display: grid;
    gap: calc(var(--space) * 2);
    margin-top: calc(var(--space) * 5);
  }
  .srow {
    display: grid;
    grid-template-columns: 1.6fr repeat(5, 1fr);
    gap: calc(var(--space) * 3);
    align-items: center;
    padding: calc(var(--space) * 2.5) calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    border: 1px solid var(--border);
  }
  .target {
    margin-left: calc(var(--space) * 2);
  }
  .srow.head {
    background: none;
    border-color: transparent;
    padding-block: 0;
  }
  .warn {
    color: var(--warn);
  }
  @media (prefers-reduced-motion: reduce) {
    .fill {
      transition: none;
    }
  }
</style>
