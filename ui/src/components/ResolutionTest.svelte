<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { CmdError } from "../lib/bindings/CmdError";
  import type { RecordProgress } from "../lib/bindings/RecordProgress";
  import type { ResView } from "../lib/bindings/ResView";
  import { tk } from "../lib/i18n";
  import { flashBg, logLocal, ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import FindingCard from "./FindingCard.svelte";
  import RecordStatus from "./RecordStatus.svelte";
  import Segmented from "./Segmented.svelte";
  import StateView from "./StateView.svelte";

  // Тест чутливості до роздільності: прогони по черзі на рідній і зниженій (ABBA…), скільки
  // треба для певного висновку. Уся оцінка — на бекенді; тут лише кроки, хід запису й показ.
  // Заголовок і рамку дає батьківський блок (розкривна картка на екрані діагностики).
  let {
    game,
    hasReport = $bindable(false),
    active = $bindable(false),
    onupdated,
  }: { game: string; hasReport?: boolean; active?: boolean; onupdated: () => void } = $props();

  let view = $state<ResView | null>(null);
  let seconds = $state(30);
  let running = $state(false);
  let progress = $state<RecordProgress | null>(null);
  let error = $state<CmdError | null>(null);

  const done = $derived(view?.runs.length ?? 0);
  const nextReduced = $derived(view?.next_reduced ?? null);
  const lastReduced = $derived(view?.runs.at(-1)?.reduced ?? false);
  /** Підказка перед наступним прогоном: змінити роздільність у грі чи лишити. */
  const prompt = $derived(
    nextReduced == null || running
      ? null
      : nextReduced !== lastReduced
        ? nextReduced
          ? "ui.res.switch_now"
          : "ui.res.switch_back"
        : done > 0
          ? "ui.res.keep"
          : null,
  );
  const cmp = $derived(view?.estimate.comparisons[0]?.avg_fps ?? null);
  const pct = (v: number) => `${v >= 0 ? "+" : ""}${v.toFixed(1)}`;

  $effect(() => {
    hasReport = view?.report != null;
    active = view != null;
  });

  onMount(() => {
    void api
      .resState()
      .then((v) => {
        if (v && v.game.toLowerCase() === game.toLowerCase()) {
          view = v;
          running = v.running;
          if (running) ui.measuring = true;
        }
      })
      .catch(() => undefined);
    const sub = on("res-progress", (p) => (progress = p));
    return () => void sub.then((u) => u());
  });

  async function runNext() {
    error = null;
    progress = null;
    running = true;
    ui.measuring = true;
    try {
      view = await api.resRunNext();
      if (view.report) {
        flashBg(view.report.outcome === "helps" ? "warn" : "ok");
        logLocal("info", tk("ui.res.done_log", { game: view.game }));
        onupdated();
      }
    } catch (e) {
      error = asCmdError(e);
    } finally {
      running = false;
      ui.measuring = false;
    }
  }

  async function begin() {
    error = null;
    try {
      view = await api.resStart(game, seconds);
      await runNext();
    } catch (e) {
      error = asCmdError(e);
    }
  }

  async function close() {
    await api.resCancel();
    view = null;
    running = false;
    ui.measuring = false;
    progress = null;
    error = null;
    onupdated();
  }

  const fps = (v: number) => v.toFixed(1);
</script>

<div class="res">
  {#if !view}
    <div class="setup">
      <p class="small muted">{tk("ui.res.how")}</p>
      <div class="duration">
        <span class="small muted">{tk("ui.res.run_length")}</span>
        <Segmented
          label={tk("ui.res.run_length")}
          bind:value={seconds}
          options={[
            { value: 30, label: "30 с" },
            { value: 60, label: "60 с" },
          ]}
        />
      </div>
    </div>
    <div class="actions">
      <Button variant="primary" icon="play" onclick={begin}>{tk("ui.res.start")}</Button>
    </div>
  {:else}
    <ol class="steps">
      {#each view.runs as r, i (i)}
        <li class="done">
          <span class="n mono">{i + 1}</span>
          <span>{r.reduced ? tk("ui.res.step_reduced") : tk("ui.res.step_native")}</span>
          <span class="mono muted">{tk("ui.res.run_value", { avg: fps(r.avg_fps), low: fps(r.low_1pct_fps) })}</span>
        </li>
      {/each}
      {#if nextReduced != null}
        <li class="current">
          <span class="n mono">{done + 1}</span>
          <span>{nextReduced ? tk("ui.res.step_reduced") : tk("ui.res.step_native")}</span>
        </li>
      {/if}
    </ol>

    {#if running}
      <RecordStatus {progress} game={view.game} />
    {:else if view.report}
      <FindingCard finding={view.report.finding} />
    {:else if prompt}
      <p class="switch" class:keep={prompt === "ui.res.keep"} role="status">{tk(prompt, { n: done + 1 })}</p>
    {/if}
    {#if !view.report && cmp}
      <p class="small muted">
        {tk("ui.res.now", { delta: pct(cmp.delta_pct), lo: pct(cmp.ci_low_pct), hi: pct(cmp.ci_high_pct) })}
        {#if view.estimate.more_per_arm != null}
          · {tk("ui.res.more", { n: view.estimate.more_per_arm, max: view.max_per_arm })}
        {/if}
      </p>
    {/if}

    {#if error}
      <StateView kind="error" title={tk("ui.common.error")} text={error.message} />
    {/if}

    <div class="actions">
      {#if view.report}
        <Button variant="secondary" onclick={close}>{tk("ui.res.close")}</Button>
      {:else}
        <Button variant="secondary" icon="stop" onclick={close}>{tk("ui.res.cancel")}</Button>
        <Button variant="primary" icon="play" loading={running} disabled={running} onclick={runNext}>
          {tk("ui.res.run", { n: done + 1 })}
        </Button>
      {/if}
    </div>
  {/if}
</div>

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
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
    margin-top: calc(var(--space) * 4);
  }
  .steps {
    display: grid;
    gap: calc(var(--space) * 2);
    margin: 0 0 calc(var(--space) * 4);
    padding: 0;
    list-style: none;
  }
  .steps li {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 2) calc(var(--space) * 3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    color: var(--text-muted);
    font-size: 13px;
  }
  .steps li .mono:last-child {
    margin-left: auto;
  }
  .steps li.current {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .steps li.done {
    color: var(--text);
  }
  .steps li.done .n {
    color: var(--ok);
  }
  .n {
    width: calc(var(--space) * 5);
    text-align: center;
  }
  .switch.keep {
    border-color: var(--border-strong);
  }
  .switch {
    margin: 0 0 calc(var(--space) * 3);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--warn);
    border-radius: var(--radius-sm);
    color: var(--text);
    font-size: 13px;
  }
</style>
