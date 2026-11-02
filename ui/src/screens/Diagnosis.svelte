<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { CmdError } from "../lib/bindings/CmdError";
  import type { DiagnosisResult } from "../lib/bindings/DiagnosisResult";
  import type { RecordProgress } from "../lib/bindings/RecordProgress";
  import { tk } from "../lib/i18n";
  import { flashBg, logLocal, ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import Fold from "../components/Fold.svelte";
  import CrashView from "../components/CrashView.svelte";
  import Page from "../components/Page.svelte";
  import ProcessPicker from "../components/ProcessPicker.svelte";
  import RecordHistory from "../components/RecordHistory.svelte";
  import RecordStatus from "../components/RecordStatus.svelte";
  import ResultView from "../components/ResultView.svelte";
  import LagWatch from "../components/LagWatch.svelte";
  import ResolutionTest from "../components/ResolutionTest.svelte";
  import StartModePicker from "../components/StartModePicker.svelte";
  import Segmented from "../components/Segmented.svelte";
  import StateView from "../components/StateView.svelte";
  import VerdictGuide from "../components/VerdictGuide.svelte";
  import { currentStep, wantsExample } from "../lib/tour.svelte";
  import { EXAMPLE_RESULT } from "../lib/tourExample";

  /** Вкладка: «Чому лагає?» (запис гри), «Щойно лагнуло?» (стеження з буфером) чи «Чому вилітає?». */
  let tab = $state<"lag" | "watch" | "history" | "crash">("lag");
  /** Налаштування запису згортаються, коли є результат: зверху має бути відповідь. */
  let setupOpen = $state(true);
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

  /** Тест роздільності має сенс, коли вузьке місце — GPU чи CPU (а не ліміт кадрів). */
  const modeLabel = $derived(tk(ui.settings.record_start === "hotkey" ? "ui.rec.mode.hotkey" : "ui.rec.mode.auto"));
  const setupSummary = $derived(`${exe.trim() || tk("ui.diag.no_game")} · ${seconds} с · ${modeLabel}`);
  const resTestable = $derived(
    !recording && !!result && (result.diagnosis.bottleneck === "gpu" || result.diagnosis.bottleneck === "cpu"),
  );
</script>

<Page
  title={tk(`ui.diag.page.${tab}.title`)}
  subtitle={tk(`ui.diag.page.${tab}.subtitle`)}
>
  <div class="tabs">
    <Segmented
      label={tk("ui.crash.tabs")}
      bind:value={tab}
      options={[
        { value: "lag", label: tk("ui.crash.tab_lag") },
        { value: "watch", label: tk("ui.crash.tab_watch") },
        { value: "history", label: tk("ui.history.tab") },
        { value: "crash", label: tk("ui.crash.tab_crash") },
      ]}
    />
    <span data-tour="diag-help">
      <Button small variant="secondary" icon="help" onclick={() => (guideOpen = true)}>{tk("ui.guide.open")}</Button>
    </span>
  </div>
  {#if tab === "crash"}
    <CrashView />
  {:else if tab === "history"}
    <RecordHistory />
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

      {#if shown}
        <ResultView result={shown} {example} hideResFindings={resShown}>
          {#snippet after()}
            {#if resTestable && result}
              <Fold title={tk("ui.res.title")} summary={tk("ui.res.subtitle")} bind:open={resOpen} keep>
                <ResolutionTest
                  game={result.diagnosis.game}
                  bind:hasReport={resShown}
                  bind:active={resActive}
                  onupdated={reloadResult}
                />
              </Fold>
            {/if}
          {/snippet}
        </ResultView>
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
  .stack {
    display: grid;
    gap: calc(var(--space) * 4);
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
  .pm pre {
    white-space: pre-wrap;
  }
</style>
