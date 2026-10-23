<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { CmdError } from "../lib/bindings/CmdError";
  import type { DiagnosisResult } from "../lib/bindings/DiagnosisResult";
  import type { RecordProgress } from "../lib/bindings/RecordProgress";
  import { t, tk } from "../lib/i18n";
  import { flashBg, logLocal, ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import FindingCard from "../components/FindingCard.svelte";
  import Page from "../components/Page.svelte";
  import ProcessPicker from "../components/ProcessPicker.svelte";
  import RecordStatus from "../components/RecordStatus.svelte";
  import StartModePicker from "../components/StartModePicker.svelte";
  import Segmented from "../components/Segmented.svelte";
  import StateView from "../components/StateView.svelte";

  let exe = $state("");
  let seconds = $state(60);
  let recording = $state(false);
  let progress = $state<RecordProgress | null>(null);
  let result = $state<DiagnosisResult | null>(null);
  let error = $state<CmdError | null>(null);

  onMount(() => {
    const subs = [
      on("diag-progress", (p) => (progress = p)),
      on("diag-done", (r) => {
        result = r;
        recording = false;
        ui.measuring = false;
        const actionable = r.diagnosis.verdicts.some((v) => v.finding.impact !== "info");
        flashBg(actionable ? "warn" : "ok");
        logLocal("info", tk("ui.diag.done_log", { game: r.diagnosis.game }));
      }),
      on("diag-error", (e) => {
        error = e;
        recording = false;
        ui.measuring = false;
        flashBg("warn");
      }),
    ];
    return () => subs.forEach((p) => void p.then((u) => u()));
  });

  async function start() {
    error = null;
    result = null;
    try {
      await api.diagStart(exe.trim(), seconds);
      recording = true;
      ui.measuring = true;
      progress = null;
    } catch (e) {
      error = asCmdError(e);
    }
  }

  const stats = $derived(result?.diagnosis.stats ?? null);
  const summary = $derived(result?.diagnosis.verdicts[0]?.finding ?? null);
  const rest = $derived(result?.diagnosis.verdicts.slice(1) ?? []);
</script>

{#snippet openUpscale()}
  <Button small variant="secondary" icon="upscale" onclick={() => (ui.screen = "upscale")}>{tk("ui.up.open")}</Button>
{/snippet}

<Page title={tk("ui.diag.title")} subtitle={tk("ui.diag.subtitle")}>
  <Card title={tk("ui.diag.setup")} subtitle={tk("ui.diag.setup_hint")}>
    <div class="setup">
      <ProcessPicker bind:value={exe} label={tk("ui.diag.game")} />
      <div class="duration">
        <span class="small muted">{tk("ui.diag.duration")}</span>
        <Segmented
          label={tk("ui.diag.duration")}
          bind:value={seconds}
          options={[
            { value: 30, label: "30 с" },
            { value: 60, label: "60 с" },
            { value: 120, label: "120 с" },
          ]}
        />
      </div>
    </div>
    <div class="mode"><StartModePicker /></div>
  </Card>

  {#if recording}
    <Card><RecordStatus {progress} game={exe} /></Card>
  {:else if error?.kind === "no_frames"}
    <Card>
      <StateView kind="error" title={tk("ui.diag.no_frames_title")} text={tk("ui.diag.no_frames_text")} />
      {#if error.message}
        <details class="pm small muted"><summary>{tk("ui.diag.pm_messages")}</summary><pre class="mono">{error.message}</pre></details>
      {/if}
    </Card>
  {:else if error?.kind === "not_started"}
    <Card>
      <StateView kind="error" title={tk("ui.rec.not_started_title")} text={tk(`ui.rec.not_started.${error.message || "wait_timeout"}`)} />
    </Card>
  {:else if error}
    <Card><StateView kind="error" title={tk("ui.common.error")} text={error.message} /></Card>
  {:else if !result}
    <Card><StateView kind="empty" title={tk("ui.diag.empty_title")} text={tk("ui.diag.empty_text")} /></Card>
  {/if}

  {#if result && summary}
    <Card title={t(summary.title)} subtitle={t(summary.detail)}>
      {#if stats}
        <div class="stats">
          <div><strong>{stats.avg_fps.toFixed(1)}</strong><span>{tk("ui.diag.avg")}</span></div>
          <div><strong>{stats.low_1pct_fps.toFixed(1)}</strong><span>1% low</span></div>
          <div><strong>{stats.low_01pct_fps.toFixed(1)}</strong><span>0.1% low</span></div>
          <div><strong>{stats.p99_frametime_ms.toFixed(2)}</strong><span>{tk("ui.diag.p99")}</span></div>
        </div>
      {/if}
      <p class="small dim">
        {tk("ui.diag.confidence", { v: summary.confidence })}
        {#if result.display_hz} · {tk("ui.diag.hz", { hz: result.display_hz })}{/if}
        · {tk("ui.rec.counted_total", { s: Math.round(result.counted_s) })}
        {#if result.end_reason !== "completed"} · {tk(`ui.rec.end.${result.end_reason}`)}{/if}
      </p>
    </Card>
    <div class="list">
      {#each rest as v, i (i)}
        <FindingCard finding={v.finding} action={v.finding.id === "upscale.helps" ? openUpscale : undefined} />
      {/each}
    </div>
    {#if result.presentmon_messages.length}
      <details class="pm small muted">
        <summary>{tk("ui.diag.pm_messages")}</summary>
        <pre class="mono">{result.presentmon_messages.join("\n")}</pre>
      </details>
    {/if}
  {/if}

  {#snippet secondary()}
    {#if recording}
      <Button variant="secondary" icon="stop" onclick={() => api.diagCancel()}>{tk("ui.diag.stop")}</Button>
    {/if}
  {/snippet}
  {#snippet primary()}
    <Button variant="primary" icon="play" disabled={!exe.trim() || recording} loading={recording} onclick={start}>
      {tk("ui.diag.start")}
    </Button>
  {/snippet}
</Page>

<style>
  .setup {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: calc(var(--space) * 6);
    align-items: start;
  }
  .duration {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .mode {
    margin-top: calc(var(--space) * 5);
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: calc(var(--space) * 3);
    margin-bottom: calc(var(--space) * 3);
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
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .pm pre {
    white-space: pre-wrap;
  }
</style>
