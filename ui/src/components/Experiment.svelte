<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { CmdError } from "../lib/bindings/CmdError";
  import type { ExpComparison } from "../lib/bindings/ExpComparison";
  import type { ExpRecord } from "../lib/bindings/ExpRecord";
  import type { ExpReport } from "../lib/bindings/ExpReport";
  import type { ExpView } from "../lib/bindings/ExpView";
  import type { RecordProgress } from "../lib/bindings/RecordProgress";
  import { dateTime } from "../lib/format";
  import { t, tk } from "../lib/i18n";
  import { flashBg, logLocal, ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import ExpEstimates from "./ExpEstimates.svelte";
  import ProcessPicker from "./ProcessPicker.svelte";
  import RecordStatus from "./RecordStatus.svelte";
  import Segmented from "./Segmented.svelte";
  import StateView from "./StateView.svelte";
  import Toggle from "./Toggle.svelte";

  // Порівняння 2–4 варіантів будь-якої зміни (науковий A/B). Порядок прогонів, оцінку й
  // рішення «досить чи ще» дає бекенд (`optimax_experiment`); тут — кроки, підказки й показ.
  let { game = $bindable("") }: { game?: string } = $props();

  const MAX_VARIANTS = 4;
  let names = $state<string[]>(["", ""]);
  let seconds = $state(30);
  let warmup = $state(true);
  let view = $state<ExpView | null>(null);
  let progress = $state<RecordProgress | null>(null);
  let busy = $state(false);
  let error = $state<CmdError | null>(null);
  let saved = $state<ExpRecord[]>([]);

  const running = $derived(view?.phase === "running");
  const report = $derived(view?.report ?? null);
  const letter = (v: number) => String.fromCharCode(65 + v);
  const nameOf = (v: number, list: string[]) => list[v] ?? letter(v);
  const included = $derived(report ? report.runs.filter((r) => !r.excluded) : []);
  const nextArm = $derived(report?.next ?? null);
  /** Варіант треба перемкнути в грі перед наступним прогоном. */
  const mustSwitch = $derived(
    nextArm != null && included.length > 0 && included[included.length - 1]?.variant !== nextArm,
  );

  $effect(() => {
    ui.measuring = view?.phase === "running" || view?.phase === "warmup";
  });

  async function loadSaved() {
    try {
      saved = await api.expList();
    } catch {
      saved = [];
    }
  }

  onMount(() => {
    void api
      .expState()
      .then((v) => {
        if (v) {
          view = v;
          game = v.game;
        }
      })
      .catch(() => undefined);
    void loadSaved();
    const subs = [on("exp-progress", (p) => (progress = p)), on("exp-update", (v) => (view = v))];
    return () => {
      subs.forEach((p) => void p.then((u) => u()));
      ui.measuring = false;
    };
  });

  async function begin() {
    error = null;
    try {
      view = await api.expStart(game.trim(), names, seconds, warmup);
    } catch (e) {
      error = asCmdError(e);
    }
  }

  async function runNext() {
    if (!view) return;
    error = null;
    progress = null;
    busy = true;
    view = { ...view, phase: "running" };
    try {
      view = await api.expRunNext();
      if (view.phase === "done") {
        flashBg(view.report.comparisons.some((c) => c.verdict === "worse") ? "warn" : "ok");
        logLocal("info", tk("ui.exp.done_log", { game: view.game }));
        void loadSaved();
      }
    } catch (e) {
      error = asCmdError(e);
      view = await api.expState().catch(() => view);
    } finally {
      busy = false;
    }
  }

  async function skipWarmup() {
    try {
      view = await api.expSkipWarmup();
    } catch (e) {
      error = asCmdError(e);
    }
  }

  async function exclude(index: number, excluded: boolean) {
    try {
      view = await api.expExclude(index, excluded);
      if (view.phase === "done") void loadSaved();
    } catch (e) {
      error = asCmdError(e);
    }
  }

  async function close() {
    await api.expClose();
    view = null;
    progress = null;
    error = null;
    void loadSaved();
  }

  async function remove(id: string) {
    try {
      await api.expDelete(id);
    } finally {
      void loadSaved();
    }
  }

  const pct = (v: number) => `${v >= 0 ? "+" : ""}${v.toFixed(1)}`;
  const fps = (v: number) => v.toFixed(1);

  /** Рядок висновку одного порівняння (той самий для активного й збереженого). */
  function verdictText(c: ExpComparison, list: string[]): string {
    const avg = c.avg_fps;
    return tk(`ui.exp.verdict.${c.verdict}`, {
      a: nameOf(0, list),
      b: nameOf(c.variant, list),
      d: avg ? pct(avg.delta_pct) : "—",
      lo: avg ? pct(avg.ci_low_pct) : "—",
      hi: avg ? pct(avg.ci_high_pct) : "—",
      low: c.low_1pct ? pct(c.low_1pct.delta_pct) : "—",
    });
  }

  /** Найгірший висновок серед порівнянь — для кольору рамки й рядка. */
  function worst(r: ExpReport): string {
    const v = r.comparisons.map((c) => c.verdict);
    return v.includes("worse") ? "worse" : v.includes("better") ? "better" : (v[0] ?? "unclear");
  }

  function title(list: string[]): string {
    return tk("ui.exp.title", {
      a: nameOf(0, list),
      b: list
        .slice(1)
        .map((n) => `«${n}»`)
        .join(", "),
    });
  }
</script>

<div class="exp">
  {#if !view}
    <Card title={tk("ui.exp.setup_title")} subtitle={tk("ui.exp.setup_text")}>
      <div class="grid">
        <ProcessPicker bind:value={game} label={tk("ui.diag.game")} />
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
      <div class="names">
        {#each names as _, i (i)}
          <div class="field">
            <label class="small muted" for="exp-name-{i}">
              {i === 0 ? tk("ui.exp.name_a") : tk("ui.exp.name_n", { l: letter(i) })}
            </label>
            <div class="name-row">
              <input
                id="exp-name-{i}"
                bind:value={names[i]}
                maxlength="32"
                placeholder={tk(i === 0 ? "ui.exp.name_a_ph" : i === 1 ? "ui.exp.name_b_ph" : "ui.exp.name_more_ph")}
                autocomplete="off"
              />
              {#if i >= 2}
                <Button
                  small
                  variant="ghost"
                  icon="close"
                  label={tk("ui.exp.remove_variant", { l: letter(i) })}
                  onclick={() => names.splice(i, 1)}
                />
              {/if}
            </div>
          </div>
        {/each}
      </div>
      {#if names.length < MAX_VARIANTS}
        <div class="add">
          <Button small variant="secondary" icon="plus" onclick={() => names.push("")}>
            {tk("ui.exp.add_variant", { l: letter(names.length) })}
          </Button>
        </div>
      {/if}
      <div class="row">
        <Toggle bind:checked={warmup} label={tk("ui.exp.warmup")} />
        <span class="small">{tk("ui.exp.warmup")}</span>
        <span class="small dim">{tk("ui.exp.warmup_hint")}</span>
      </div>
      <p class="small dim">{tk(names.length > 2 ? "ui.exp.how_many" : "ui.exp.how", { n: names.length })}</p>
      {#if error}<StateView kind="error" title={tk("ui.common.error")} text={error.message} />{/if}
      <div class="actions">
        <Button variant="primary" icon="play" disabled={!game.trim()} onclick={begin}>{tk("ui.exp.start")}</Button>
      </div>
    </Card>

    {#if saved.length}
      <Card title={tk("ui.exp.saved_title")}>
        <ul class="saved">
          {#each saved as r (r.id)}
            <li>
              <div>
                <strong>{r.game}</strong>
                <span class="small muted">· {r.names.join(" · ")} · {dateTime(r.at_unix_ms / 1000)}</span>
                {#each r.report.comparisons as c (c.variant)}
                  <p class="small verdict {c.verdict}">{verdictText(c, r.names)}</p>
                {/each}
              </div>
              <Button small variant="secondary" icon="trash" onclick={() => remove(r.id)}>{tk("ui.exp.delete")}</Button>
            </li>
          {/each}
        </ul>
      </Card>
    {/if}
  {:else if report}
    <Card
      title={title(view.names)}
      subtitle={tk("ui.exp.subtitle", { game: view.game, s: view.seconds, max: view.max_per_arm })}
    >
      {#if view.phase === "warmup" && view.warmup}
        {@const w = view.warmup}
        <div class="box" role="status" aria-live="polite">
          <strong>{tk("ui.exp.warmup_now")}</strong>
          <span class="small">{tk("ui.exp.warmup_play", { name: nameOf(0, view.names) })}</span>
          <span class="small mono muted">
            {#if w.has_temp && w.thermal.last_c != null}
              {tk("ui.exp.warmup_temp", {
                c: w.thermal.last_c.toFixed(0),
                slope: w.thermal.slope_c_per_min != null ? pct(w.thermal.slope_c_per_min) : "—",
              })} ·
            {:else}
              {tk("ui.exp.warmup_no_temp")} ·
            {/if}
            {tk("ui.exp.warmup_time", { s: Math.floor(w.elapsed_s), max: Math.round(w.max_s) })}
          </span>
          <div class="progress"><i style:width="{Math.min(100, (w.elapsed_s / w.max_s) * 100)}%"></i></div>
        </div>
      {:else if view.phase === "running"}
        <RecordStatus {progress} game={view.game} />
        {#if nextArm != null}
          <p class="small muted">{tk("ui.exp.running_arm", { name: nameOf(nextArm, view.names) })}</p>
        {/if}
      {:else if view.phase === "ready" && nextArm != null}
        <div class="box" class:switch={mustSwitch} role="status">
          <strong>
            {tk(mustSwitch ? "ui.exp.switch_to" : "ui.exp.keep", {
              name: nameOf(nextArm, view.names),
              n: report.runs.length + 1,
            })}
          </strong>
          <span class="small muted">{tk("ui.exp.same_scene")}</span>
        </div>
      {:else if view.phase === "done"}
        <div class="box done {worst(report)}" role="status">
          {#each report.comparisons as c (c.variant)}<strong>{verdictText(c, view.names)}</strong>{/each}
          {#if view.saved_id}<span class="small dim">{tk("ui.exp.saved")}</span>{/if}
        </div>
      {/if}

      <ExpEstimates {report} names={view.names} />
      {#if view.phase !== "done" && report.more_per_arm != null}
        <p class="small muted">{tk("ui.exp.more_runs", { n: report.more_per_arm })}</p>
      {/if}

      {#if report.runs.length}
        <table class="runs small">
          <thead>
            <tr>
              <th>#</th>
              <th>{tk("ui.exp.col.variant")}</th>
              <th>{tk("ui.exp.col.avg")}</th>
              <th>1% low</th>
              <th>{tk("ui.exp.col.precision")}</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each report.runs as r, i (i)}
              <tr class:excluded={r.excluded}>
                <td class="mono">{r.order + 1}</td>
                <td><span class="arm" class:base={r.variant === 0}>{letter(r.variant)}</span> {nameOf(r.variant, view.names)}</td>
                <td class="mono">{fps(r.avg_fps)}</td>
                <td class="mono">{fps(r.low_1pct_fps)}</td>
                <td class="mono dim">{r.low_precision_pct != null ? `±${r.low_precision_pct.toFixed(1)} %` : "—"}</td>
                <td class="act">
                  {#if r.suspect && !r.excluded}<span class="suspect small">{tk(`ui.exp.suspect.${r.suspect}`)}</span>{/if}
                  <Button small variant="secondary" disabled={running} onclick={() => exclude(i, !r.excluded)}>
                    {tk(r.excluded ? "ui.exp.include" : "ui.exp.exclude")}
                  </Button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}

      {#if report.warnings.length}
        <ul class="warns small">
          {#each report.warnings as w, i (i)}<li>{t(w)}</li>{/each}
        </ul>
      {/if}
      {#if error}<StateView kind="error" title={tk("ui.common.error")} text={error.message} />{/if}

      <div class="actions">
        <Button variant="secondary" icon={view.phase === "done" ? undefined : "stop"} onclick={close}>
          {tk(view.phase === "done" ? "ui.exp.close" : "ui.exp.cancel")}
        </Button>
        {#if view.phase === "warmup"}
          <Button variant="secondary" onclick={skipWarmup}>{tk("ui.exp.skip_warmup")}</Button>
        {/if}
        {#if view.phase !== "done"}
          <Button
            variant="primary"
            icon="play"
            loading={busy}
            disabled={busy || view.phase !== "ready"}
            onclick={runNext}
          >
            {tk("ui.exp.run", { n: report.runs.length + 1 })}
          </Button>
        {/if}
      </div>
    </Card>
  {/if}
</div>

<style>
  .exp {
    display: grid;
    gap: calc(var(--space) * 4);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: calc(var(--space) * 6);
    align-items: start;
  }
  .duration {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .names {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: calc(var(--space) * 4);
    margin-top: calc(var(--space) * 5);
  }
  .field {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .name-row {
    display: flex;
    gap: calc(var(--space) * 2);
  }
  .name-row input {
    flex: 1;
    min-width: 0;
  }
  .add {
    margin-top: calc(var(--space) * 3);
  }
  .field input {
    height: 38px;
    padding: 0 calc(var(--space) * 3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    font-size: 13px;
  }
  .field input:focus-visible {
    border-color: var(--accent-from);
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: calc(var(--space) * 3);
    margin-top: calc(var(--space) * 5);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
    margin-top: calc(var(--space) * 4);
  }
  .box {
    display: grid;
    gap: calc(var(--space) * 1);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    margin-bottom: calc(var(--space) * 4);
  }
  .box.switch {
    border-color: var(--warn);
  }
  .box.done.better {
    border-color: var(--ok);
  }
  .box.done.worse {
    border-color: var(--danger);
  }
  .progress {
    height: 4px;
    border-radius: 2px;
    background: var(--border);
    overflow: hidden;
  }
  .progress i {
    display: block;
    height: 100%;
    background: linear-gradient(90deg, var(--accent-from), var(--accent-to));
  }
  .runs {
    width: 100%;
    border-collapse: collapse;
    margin-top: calc(var(--space) * 4);
  }
  .runs th {
    text-align: left;
    font-weight: 500;
    color: var(--text-muted);
    padding: calc(var(--space) * 1) calc(var(--space) * 2);
    border-bottom: 1px solid var(--border);
  }
  .runs td {
    padding: calc(var(--space) * 1) calc(var(--space) * 2);
    border-bottom: 1px solid var(--border);
  }
  .runs tr.excluded td {
    opacity: 0.45;
  }
  .act {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
  }
  .arm {
    display: inline-block;
    min-width: 18px;
    text-align: center;
    border-radius: 4px;
    font-weight: 600;
    background: var(--bg-input);
    border: 1px solid var(--border-strong);
  }
  .arm:not(.base) {
    border-color: var(--accent-from);
  }
  .suspect {
    color: var(--warn);
  }
  .warns {
    margin: calc(var(--space) * 3) 0 0;
    padding-left: calc(var(--space) * 5);
    color: var(--warn);
  }
  .saved {
    display: grid;
    gap: calc(var(--space) * 3);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .saved li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
  }
  .verdict {
    margin: calc(var(--space) * 1) 0 0;
  }
  .verdict.better {
    color: var(--ok);
  }
  .verdict.worse {
    color: var(--danger);
  }
  p {
    margin: 0;
  }
</style>
