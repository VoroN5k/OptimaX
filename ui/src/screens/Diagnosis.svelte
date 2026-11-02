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
  import Fold from "../components/Fold.svelte";
  import FrametimeChart from "../components/FrametimeChart.svelte";
  import CrashView from "../components/CrashView.svelte";
  import Page from "../components/Page.svelte";
  import ProcessPicker from "../components/ProcessPicker.svelte";
  import RecordStatus from "../components/RecordStatus.svelte";
  import LagWatch from "../components/LagWatch.svelte";
  import ResolutionTest from "../components/ResolutionTest.svelte";
  import StartModePicker from "../components/StartModePicker.svelte";
  import Segmented from "../components/Segmented.svelte";
  import StateView from "../components/StateView.svelte";
  import VerdictGuide from "../components/VerdictGuide.svelte";
  import { currentStep, wantsExample } from "../lib/tour.svelte";
  import { EXAMPLE_RESULT } from "../lib/tourExample";

  /** Вкладка: «Чому лагає?» (запис гри), «Щойно лагнуло?» (стеження з буфером) чи «Чому вилітає?». */
  let tab = $state<"lag" | "watch" | "crash">("lag");
  /** Налаштування запису згортаються, коли є результат: зверху має бути відповідь. */
  let setupOpen = $state(true);
  let chartOpen = $state(false);
  let resOpen = $state(false);
  let resActive = $state(false);
  // Тест роздільності, що вже йде або має підсумок, не ховаємо.
  $effect(() => {
    if (resActive) resOpen = true;
  });
  let exe = $state("");
  let seconds = $state(60);
  let recording = $state(false);
  let progress = $state<RecordProgress | null>(null);
  let result = $state<DiagnosisResult | null>(null);
  let error = $state<CmdError | null>(null);

  onMount(() => {
    // Повернення на сторінку: показати останній результат і хід запису, якщо він іде.
    void api
      .diagState()
      .then((s) => {
        if (s.recording) {
          recording = true;
          ui.measuring = true;
        }
        if (s.last && !result) {
          result = s.last;
          setupOpen = false;
          if (!exe) exe = s.last.diagnosis.game;
        }
      })
      .catch(() => undefined);
    const subs = [
      on("diag-progress", (p) => (progress = p)),
      on("diag-done", (r) => {
        result = r;
        setupOpen = false;
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

  /** Підсумок тесту роздільності вже показано в його картці — у списку не дублюємо. */
  let resShown = $state(false);

  /** Бекенд уточнив діагноз (тест роздільності) — перечитати. */
  async function reloadResult() {
    try {
      const s = await api.diagState();
      if (s.last) result = s.last;
    } catch {
      /* лишаємо показаний результат */
    }
  }

  /** Знайомство (розділ 3) без власного запису — позначений приклад, нікуди не зберігається. */
  const example = $derived(wantsExample() && !result && !recording);
  const shown = $derived(result ?? (example ? EXAMPLE_RESULT : null));
  let guideOpen = $state(false);

  // Знайомство веде сюди: розділ 2 — розгорнуті налаштування запису, розділ 3 — результат.
  $effect(() => {
    const s = currentStep();
    if (!s || s.screen !== "diagnosis") return;
    tab = "lag";
    setupOpen = s.chapter === 2;
  });

  const stats = $derived(shown?.diagnosis.stats ?? null);
  const summary = $derived(shown?.diagnosis.verdicts[0]?.finding ?? null);
  /** Цифри, що вже стоять плитками вище, — у списку доказів підсумку не повторюємо. */
  const TILE_KEYS = new Set(["diag.ev.avg_fps", "diag.ev.low1", "diag.ev.low01", "diag.ev.p99"]);
  const extra = $derived((summary?.evidence ?? []).filter((e) => !TILE_KEYS.has(e.label.key)));
  const rest = $derived(
    (shown?.diagnosis.verdicts.slice(1) ?? []).filter((v) => !(resShown && v.finding.id.startsWith("res."))),
  );
  /** Тест роздільності має сенс, коли вузьке місце — GPU чи CPU (а не ліміт кадрів). */
  const modeLabel = $derived(tk(ui.settings.record_start === "hotkey" ? "ui.rec.mode.hotkey" : "ui.rec.mode.auto"));
  const setupSummary = $derived(`${exe.trim() || tk("ui.diag.no_game")} · ${seconds} с · ${modeLabel}`);
  const resTestable = $derived(
    !recording && !!result && (result.diagnosis.bottleneck === "gpu" || result.diagnosis.bottleneck === "cpu"),
  );
</script>

{#snippet openUpscale()}
  <Button small variant="secondary" icon="upscale" onclick={() => (ui.screen = "upscale")}>{tk("ui.up.open")}</Button>
{/snippet}

<Page
  title={tab === "crash" ? tk("ui.crash.title") : tab === "watch" ? tk("ui.lag.title") : tk("ui.diag.title")}
  subtitle={tab === "crash" ? tk("ui.crash.subtitle") : tab === "watch" ? tk("ui.lag.subtitle") : tk("ui.diag.subtitle")}
>
  <div class="tabs">
    <Segmented
      label={tk("ui.crash.tabs")}
      bind:value={tab}
      options={[
        { value: "lag", label: tk("ui.crash.tab_lag") },
        { value: "watch", label: tk("ui.crash.tab_watch") },
        { value: "crash", label: tk("ui.crash.tab_crash") },
      ]}
    />
    <span data-tour="diag-help">
      <Button small variant="secondary" icon="help" onclick={() => (guideOpen = true)}>{tk("ui.guide.open")}</Button>
    </span>
  </div>
  {#if tab === "crash"}
    <CrashView />
  {:else if tab === "watch"}
    <div class="stack">
      <Card>
        <ProcessPicker bind:value={exe} label={tk("ui.diag.game")} />
      </Card>
      <LagWatch game={exe} heading={false} />
    </div>
  {:else}
    <div class="stack">
      <Fold title={tk("ui.diag.setup")} summary={setupOpen ? tk("ui.diag.setup_hint") : setupSummary} bind:open={setupOpen}>
        <div class="setup">
          <div data-tour="diag-game"><ProcessPicker bind:value={exe} label={tk("ui.diag.game")} /></div>
          <div class="duration" data-tour="diag-duration">
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
        <div class="mode" data-tour="diag-start-mode"><StartModePicker /></div>
      </Fold>

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
      {:else if !shown}
        <Card><StateView kind="empty" title={tk("ui.diag.empty_title")} text={tk("ui.diag.empty_text")} /></Card>
      {/if}

      {#if shown && summary}
        {#if example}
          <div class="example" role="note">
            <strong>{tk("ui.tour.example_title")}</strong>
            <span>{tk("ui.tour.example_text")}</span>
          </div>
        {/if}
        <div data-tour="diag-summary">
        <Card title={t(summary.title)} subtitle={t(summary.detail)}>
          {#if stats}
            <div class="stats" data-tour="diag-stats">
              <div><strong>{stats.avg_fps.toFixed(1)}</strong><span>{tk("ui.diag.avg")}</span></div>
              <div><strong>{stats.low_1pct_fps.toFixed(1)}</strong><span>1% low</span></div>
              <div><strong>{stats.low_01pct_fps.toFixed(1)}</strong><span>0.1% low</span></div>
              <div><strong>{stats.p99_frametime_ms.toFixed(2)}</strong><span>{tk("ui.diag.p99")}</span></div>
            </div>
          {/if}
          {#if extra.length}
            <p class="small muted load-title">{tk("ui.diag.load")}</p>
            <dl class="load">
              {#each extra as e, i (i)}
                <dt>{t(e.label)}</dt>
                <dd class="mono">{t(e.value)}</dd>
              {/each}
            </dl>
          {/if}
          <p class="small dim">
            {tk("ui.diag.confidence", { v: summary.confidence })}
            {#if shown.display_hz} · {tk("ui.diag.hz", { hz: shown.display_hz })}{/if}
            · {tk("ui.rec.counted_total", { s: Math.round(shown.counted_s) })}
            {#if shown.end_reason !== "completed"} · {tk(`ui.rec.end.${shown.end_reason}`)}{/if}
          </p>
        </Card>
        </div>

        <section class="findings" data-tour="diag-findings" aria-label={tk("ui.diag.findings")}>
          <h2 class="section muted">{tk("ui.diag.findings_n", { n: rest.length })}</h2>
          {#if rest.length}
            <div class="list">
              {#each rest as v, i (`${shown.diagnosis.game}-${i}-${v.finding.id}`)}
                <FindingCard
                  finding={v.finding}
                  collapsible
                  expanded={i === 0}
                  action={v.finding.id === "upscale.helps" ? openUpscale : undefined}
                />
              {/each}
            </div>
          {:else}
            <p class="none small muted">{tk("ui.diag.no_more")}</p>
          {/if}
        </section>

        {#if shown.diagnosis.chart}
          <div data-tour="diag-radar">
          <Fold
            title={tk("ui.chart.title")}
            summary={tk("ui.chart.spikes", { n: shown.diagnosis.chart.spikes_total })}
            bind:open={chartOpen}
          >
            <p class="small muted fold-hint">{tk("ui.chart.subtitle")}</p>
            <FrametimeChart chart={shown.diagnosis.chart} />
          </Fold>
          </div>
        {/if}
        {#if resTestable && shown}
          <Fold title={tk("ui.res.title")} summary={tk("ui.res.subtitle")} bind:open={resOpen} keep>
            <ResolutionTest
              game={shown.diagnosis.game}
              bind:hasReport={resShown}
              bind:active={resActive}
              onupdated={reloadResult}
            />
          </Fold>
        {/if}
        {#if shown.presentmon_messages.length}
          <details class="pm small muted">
            <summary>{tk("ui.diag.pm_messages")}</summary>
            <pre class="mono">{shown.presentmon_messages.join("\n")}</pre>
          </details>
        {/if}
      {/if}
    </div>
  {/if}

  {#snippet secondary()}
    {#if recording && tab === "lag"}
      <Button variant="secondary" icon="stop" onclick={() => api.diagCancel()}>{tk("ui.diag.stop")}</Button>
    {/if}
  {/snippet}
  {#snippet primary()}
    {#if tab === "lag"}
      <span data-tour="diag-start"><Button variant="primary" icon="play" disabled={!exe.trim() || recording} loading={recording} onclick={start}>
        {tk("ui.diag.start")}
      </Button></span>
    {/if}
  {/snippet}
</Page>

{#if guideOpen}
  <VerdictGuide onclose={() => (guideOpen = false)} />
{/if}

<style>
  .tabs {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
    margin-bottom: calc(var(--space) * 4);
  }
  .example {
    display: grid;
    gap: 2px;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px dashed var(--warn);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    font-size: 13px;
  }
  .example strong {
    color: var(--warn);
  }
  .example span {
    color: var(--text-muted);
  }
  .stack {
    display: grid;
    gap: calc(var(--space) * 4);
  }
  .findings {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .section {
    margin: calc(var(--space) * 1) 0 0;
    padding: 0 calc(var(--space) * 1);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: 11px;
  }
  .fold-hint {
    margin: 0 0 calc(var(--space) * 3);
  }
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
  .load-title {
    margin: 0 0 calc(var(--space) * 2);
  }
  .load {
    display: grid;
    grid-template-columns: minmax(160px, max-content) 1fr;
    gap: 4px calc(var(--space) * 4);
    margin: 0 0 calc(var(--space) * 3);
    padding: calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-size: 12px;
  }
  .load dt {
    color: var(--text-muted);
  }
  .load dd {
    margin: 0;
  }
  .none {
    margin: 0;
    padding: 0 calc(var(--space) * 1);
  }
  .pm pre {
    white-space: pre-wrap;
  }
</style>
