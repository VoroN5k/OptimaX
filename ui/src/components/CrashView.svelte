<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { CmdError } from "../lib/bindings/CmdError";
  import type { CrashEntryKind } from "../lib/bindings/CrashEntryKind";
  import type { CrashReport } from "../lib/bindings/CrashReport";
  import { t, tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import FindingCard from "./FindingCard.svelte";
  import StateView from "./StateView.svelte";
  import Toggle from "./Toggle.svelte";

  // «Чому вилітає?»: усі вердикти й хронологію визначає бекенд із журналу подій.
  let report = $state<CrashReport | null>(null);
  let loading = $state(false);
  let error = $state<CmdError | null>(null);
  /** Скільки записів хронології показано. */
  let shown = $state(30);

  async function load() {
    loading = true;
    error = null;
    try {
      report = await api.crashReport();
      shown = 30;
    } catch (e) {
      error = asCmdError(e);
    } finally {
      loading = false;
    }
  }

  async function toggleHardOff() {
    await api.setSettings($state.snapshot(ui.settings));
    await load();
  }

  onMount(load);

  const tone: Record<CrashEntryKind, string> = {
    power_loss: "warn",
    bsod: "danger",
    app_crash: "info",
    gpu_reset: "danger",
    hardware: "danger",
  };

  const day = (s: number) =>
    new Date(s * 1000).toLocaleDateString("uk-UA", { day: "numeric", month: "long", year: "numeric" });
  const time = (s: number) => new Date(s * 1000).toLocaleTimeString("uk-UA", { hour: "2-digit", minute: "2-digit" });

  /** Хронологія по днях (місцевий час). */
  const days = $derived.by(() => {
    const out: { day: string; items: NonNullable<typeof report>["timeline"] }[] = [];
    for (const e of report?.timeline.slice(0, shown) ?? []) {
      const d = day(e.unix_s);
      const last = out[out.length - 1];
      if (last && last.day === d) last.items.push(e);
      else out.push({ day: d, items: [e] });
    }
    return out;
  });
</script>

<Card title={tk("ui.crash.summary_title", { days: report?.days ?? 90 })} subtitle={tk("ui.crash.summary_hint")}>
  {#if report}
    <div class="stats">
      <div><strong>{report.app_crashes}</strong><span>{tk("ui.crash.app")}</span></div>
      <div><strong>{report.bsods}</strong><span>{tk("ui.crash.bsod")}</span></div>
      <div><strong>{report.power_losses}</strong><span>{tk("ui.crash.power")}</span></div>
      <div><strong>{report.gpu_resets}</strong><span>{tk("ui.crash.gpu")}</span></div>
    </div>
  {/if}
  <div class="row">
    <div class="hard-off">
      <Toggle label={tk("ui.crash.hard_off")} bind:checked={ui.settings.power_off_by_button} onchange={toggleHardOff} />
      <div>
        <strong>{tk("ui.crash.hard_off")}</strong>
        <p class="small muted">{tk("ui.crash.hard_off_hint")}</p>
      </div>
    </div>
    <Button variant="secondary" icon="refresh" {loading} onclick={load}>{tk("ui.crash.refresh")}</Button>
  </div>
</Card>

{#if loading && !report}
  <Card><StateView kind="loading" title={tk("ui.crash.loading")} /></Card>
{:else if error}
  <Card><StateView kind="error" title={tk("ui.common.error")} text={error.message} /></Card>
{:else if report}
  <div class="list">
    {#each report.findings as f, i (i)}
      <FindingCard finding={f} />
    {/each}
  </div>

  <Card title={tk("ui.crash.timeline")} subtitle={tk("ui.crash.timeline_hint")}>
    {#if report.timeline.length}
      <div class="timeline">
        {#each days as d (d.day)}
          <section>
            <h4 class="small muted">{d.day}</h4>
            <ul>
              {#each d.items as e, i (i)}
                <li>
                  <span class="mono small dim">{time(e.unix_s)}</span>
                  <i class="dot {tone[e.kind]}" aria-hidden="true"></i>
                  <span>{t(e.text)}</span>
                </li>
              {/each}
            </ul>
          </section>
        {/each}
      </div>
      {#if shown < report.timeline.length}
        <button type="button" class="more small" onclick={() => (shown += 50)}>
          {tk("ui.crash.more", { n: report.timeline.length - shown })}
        </button>
      {/if}
    {:else}
      <p class="small muted">{tk("ui.crash.timeline_empty")}</p>
    {/if}
  </Card>
{/if}

<style>
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: calc(var(--space) * 3);
    margin-bottom: calc(var(--space) * 4);
  }
  .stats div {
    display: grid;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    border: 1px solid var(--border);
  }
  .stats strong {
    font-size: 24px;
    font-weight: 600;
  }
  .stats span {
    font-size: 12px;
    color: var(--text-muted);
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: calc(var(--space) * 4);
  }
  .hard-off {
    display: flex;
    gap: calc(var(--space) * 3);
    align-items: flex-start;
  }
  .hard-off strong {
    font-size: 14px;
    font-weight: 600;
  }
  .row p {
    margin: calc(var(--space) * 1) 0 0;
  }
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .timeline {
    display: grid;
    gap: calc(var(--space) * 4);
  }
  h4 {
    margin: 0 0 calc(var(--space) * 2);
    font-weight: 600;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: calc(var(--space) * 1.5);
  }
  li {
    display: grid;
    grid-template-columns: 44px 10px 1fr;
    align-items: center;
    gap: calc(var(--space) * 2);
    font-size: 13px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dot.warn {
    background: var(--warn);
  }
  .dot.danger {
    background: var(--danger);
  }
  .dot.info {
    background: var(--info);
  }
  .more {
    margin-top: calc(var(--space) * 3);
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-from);
    cursor: pointer;
  }
  .more:hover {
    text-decoration: underline;
  }
  .more:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
    border-radius: 4px;
  }
</style>
