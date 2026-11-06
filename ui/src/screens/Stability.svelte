<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { CmdError } from "../lib/bindings/CmdError";
  import type { StabInfo } from "../lib/bindings/StabInfo";
  import type { StabRecord } from "../lib/bindings/StabRecord";
  import type { StabView } from "../lib/bindings/StabView";
  import type { TestKind } from "../lib/bindings/TestKind";
  import { dateTime } from "../lib/format";
  import { tk } from "../lib/i18n";
  import { flashBg, logLocal, ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import Modal from "../components/Modal.svelte";
  import Page from "../components/Page.svelte";
  import Segmented from "../components/Segmented.svelte";
  import StateView from "../components/StateView.svelte";
  import Toggle from "../components/Toggle.svelte";

  // Лабораторія стабільності: усе рахує й вирішує бекенд (`optimax_stability`), тут лише показ.
  let info = $state<StabInfo | null>(null);
  let view = $state<StabView | null>(null);
  let records = $state<StabRecord[]>([]);
  let last = $state<StabRecord | null>(null);
  let kind = $state<TestKind>("gpu");
  let seconds = $state(600);
  let acceptNoSensor = $state(false);
  let confirming = $state(false);
  let stopping = $state(false);
  let error = $state<CmdError | null>(null);

  const noSensor = $derived(info != null && (kind === "cpu" || !info.has_sensor));
  const canStart = $derived(info != null && (!noSensor || acceptNoSensor));
  const BAD = new Set(["errors", "device_lost", "worker_crashed", "hardware_events"]);

  $effect(() => {
    ui.measuring = view != null;
  });

  async function loadRecords() {
    try {
      records = await api.stabList();
    } catch {
      records = [];
    }
  }

  onMount(() => {
    void api.stabInfo().then((i) => (info = i)).catch((e) => (error = asCmdError(e)));
    void api.stabState().then((v) => (view = v)).catch(() => undefined);
    void loadRecords();
    const subs = [
      on("stab-update", (v) => (view = v)),
      on("stab-done", (r) => {
        view = null;
        stopping = false;
        last = r;
        flashBg(BAD.has(r.outcome) ? "warn" : r.outcome === "no_errors" ? "ok" : "warn");
        logLocal("info", tk(`ui.stab.outcome.${r.outcome}.title`));
        void loadRecords();
      }),
    ];
    return () => {
      subs.forEach((p) => void p.then((u) => u()));
      ui.measuring = false;
    };
  });

  async function start() {
    confirming = false;
    error = null;
    last = null;
    try {
      view = await api.stabStart(kind, seconds, noSensor && acceptNoSensor);
    } catch (e) {
      error = asCmdError(e);
    }
  }

  async function stop() {
    stopping = true;
    await api.stabStop();
  }

  async function remove(id: string) {
    try {
      await api.stabDelete(id);
    } finally {
      if (last?.id === id) last = null;
      void loadRecords();
    }
  }

  const clock = (s: number) => {
    const t = Math.max(0, Math.floor(s));
    return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, "0")}`;
  };
  const num = (v: number | null | undefined, unit: string) =>
    v != null ? `${Math.round(v)} ${unit}` : "—";
  const durationLabel = (s: number) => tk("ui.stab.minutes", { n: s / 60 });
  /** Текст підсумку: для процесора деякі звучать інакше. */
  const outcomeText = (r: StabRecord) =>
    tk(
      r.kind === "cpu" && (r.outcome === "errors" || r.outcome === "worker_crashed")
        ? `ui.stab.outcome.${r.outcome}.text_cpu`
        : `ui.stab.outcome.${r.outcome}.text`,
      { min: (r.elapsed_s / 60).toFixed(1), n: r.mismatches, limit: info?.limit_c ?? "" },
    );
  const coreList = (cores: number[]) => cores.join(", ");
</script>

<Page title={tk("ui.stab.title")} subtitle={tk("ui.stab.subtitle")}>
  <div class="stack">
    {#if view}
      <Card title={tk("ui.stab.running_title")} subtitle={view.adapter || tk("ui.stab.starting")}>
        <div role="status" aria-live="polite" class="now">
          <strong class="mono time">{clock(view.elapsed_s)} / {clock(view.planned_s)}</strong>
          {#if view.kind === "cpu" && view.total_cores > 0}
            <span class="small muted">
              {view.current_core != null
                ? tk("ui.stab.core_now", { n: view.current_core, total: view.total_cores })
                : tk("ui.stab.core_all", { total: view.total_cores })}
            </span>
          {/if}
          <div class="progress"><i style:width="{Math.min(100, (view.elapsed_s / view.planned_s) * 100)}%"></i></div>
          {#if view.mismatches > 0}
            <p class="bad"><strong>{tk("ui.stab.found_errors", { n: view.mismatches })}</strong></p>
            {#if view.bad_cores.length}
              <p class="bad">{tk("ui.stab.bad_cores", { cores: coreList(view.bad_cores) })}</p>
            {/if}
          {/if}
        </div>
        <dl class="grid">
          <div><dt class="small muted">{tk("ui.stab.checked")}</dt><dd class="mono">{view.checked}</dd></div>
          <div><dt class="small muted">{tk("ui.stab.mismatches")}</dt><dd class="mono" class:bad={view.mismatches > 0}>{view.mismatches}</dd></div>
          {#if view.kind === "gpu"}
            <div>
              <dt class="small muted">{tk("ui.stab.temp")}</dt>
              <dd class="mono">{num(view.temp_c, "°C")}{#if view.limit_c != null}<span class="dim"> / {view.limit_c}</span>{/if}</dd>
            </div>
            <div><dt class="small muted">{tk("ui.stab.util")}</dt><dd class="mono">{num(view.util_pct, "%")}</dd></div>
            <div><dt class="small muted">{tk("ui.stab.clock")}</dt><dd class="mono">{num(view.clock_mhz, tk("ui.stab.mhz"))}</dd></div>
            <div><dt class="small muted">{tk("ui.stab.power")}</dt><dd class="mono">{num(view.power_w, tk("ui.stab.watt"))}</dd></div>
          {/if}
        </dl>
        {#if view.kind === "gpu" && view.tested_mb > 0}
          <p class="small dim">{tk("ui.stab.tested", { mb: view.tested_mb, vram: view.vram_mb })}</p>
        {/if}
        {#if view.no_sensor}<p class="small warn">{tk("ui.stab.no_sensor_running")}</p>{/if}
        <div class="actions">
          <Button variant="secondary" icon="stop" loading={stopping} disabled={stopping} onclick={stop}>{tk("ui.stab.stop")}</Button>
        </div>
      </Card>
    {:else}
      {#if last}
        {@const r = last}
        <Card title={tk(`ui.stab.outcome.${r.outcome}.title`)}>
          <div class="result" class:bad-box={BAD.has(r.outcome)} class:ok-box={r.outcome === "no_errors"} role="status">
            <p>{outcomeText(r)}</p>
          </div>
          <dl class="grid">
            <div><dt class="small muted">{tk("ui.stab.checked")}</dt><dd class="mono">{r.checked}</dd></div>
            <div><dt class="small muted">{tk("ui.stab.mismatches")}</dt><dd class="mono" class:bad={r.mismatches > 0}>{r.mismatches}</dd></div>
            {#if r.kind === "gpu"}
              <div><dt class="small muted">{tk("ui.stab.max_temp")}</dt><dd class="mono">{num(r.stats.max_temp_c, "°C")}</dd></div>
              <div><dt class="small muted">{tk("ui.stab.avg_util")}</dt><dd class="mono">{num(r.stats.avg_util_pct, "%")}</dd></div>
              <div><dt class="small muted">{tk("ui.stab.avg_clock")}</dt><dd class="mono">{num(r.stats.avg_clock_mhz, tk("ui.stab.mhz"))}</dd></div>
              <div><dt class="small muted">{tk("ui.stab.avg_power")}</dt><dd class="mono">{num(r.stats.avg_power_w, tk("ui.stab.watt"))}</dd></div>
            {:else}
              <div><dt class="small muted">{tk("ui.stab.cores_tested")}</dt><dd class="mono">{r.total_cores}</dd></div>
            {/if}
          </dl>
          <ul class="notes small">
            {#if r.kind === "gpu" && r.tested_mb > 0}<li class="dim">{tk("ui.stab.tested", { mb: r.tested_mb, vram: r.vram_mb })}</li>{/if}
            {#if r.bad_cores.length}<li class="bad">{tk("ui.stab.bad_cores", { cores: coreList(r.bad_cores) })}</li>{/if}
            {#if r.kind === "cpu" && r.outcome === "worker_crashed" && r.last_core != null}
              <li class="warn">{tk("ui.stab.crash_core", { n: r.last_core })}</li>
            {/if}
            {#if r.kind === "cpu"}<li class="dim">{tk("ui.stab.core_numbers")}</li>{/if}
            {#if r.events.tdr > 0}<li class="warn">{tk("ui.stab.ev.tdr", { n: r.events.tdr })}</li>{/if}
            {#if r.events.whea_fatal > 0}<li class="warn">{tk("ui.stab.ev.whea_fatal", { n: r.events.whea_fatal })}</li>{/if}
            {#if r.events.whea_corrected > 0}<li class="warn">{tk("ui.stab.ev.whea_corrected", { n: r.events.whea_corrected })}</li>{/if}
            {#if r.events.unexpected_off > 0}<li class="warn">{tk("ui.stab.ev.off", { n: r.events.unexpected_off })}</li>{/if}
            {#if !r.events.read}<li class="dim">{tk("ui.stab.ev.unread")}</li>{/if}
            {#if r.low_load}<li class="warn">{tk("ui.stab.low_load")}</li>{/if}
            {#if r.no_sensor}<li class="warn">{tk("ui.stab.no_sensor_running")}</li>{/if}
          </ul>
        </Card>
      {/if}

      <Card title={tk(`ui.stab.setup.${kind}.title`)} subtitle={tk(`ui.stab.setup.${kind}.text`)}>
        {#if info}
          <div class="duration">
            <span class="small muted">{tk("ui.stab.kind")}</span>
            <Segmented
              label={tk("ui.stab.kind")}
              bind:value={kind}
              options={[
                { value: "gpu", label: tk("ui.stab.kind_gpu") },
                { value: "cpu", label: tk("ui.stab.kind_cpu") },
              ]}
            />
          </div>
          <p class="small muted">
            {#if kind === "cpu"}
              {tk("ui.stab.cpu", { name: info.cpu, cores: info.cpu_cores })}
            {:else}
              {info.gpu ? tk("ui.stab.gpu", { name: info.gpu }) : tk("ui.stab.gpu_unknown")}
            {/if}
          </p>
          <div class="duration">
            <span class="small muted">{tk("ui.stab.duration")}</span>
            <Segmented
              label={tk("ui.stab.duration")}
              bind:value={seconds}
              options={info.durations_s.map((s) => ({ value: s, label: durationLabel(s) }))}
            />
          </div>
          <p class="small dim">{tk("ui.stab.duration_hint")}</p>
          {#if noSensor}
            <div class="box-warn">
              <p class="small">{tk(kind === "cpu" ? "ui.stab.no_sensor_cpu" : "ui.stab.no_sensor")}</p>
              <div class="row">
                <Toggle bind:checked={acceptNoSensor} label={tk("ui.stab.accept_no_sensor")} />
                <span class="small">{tk("ui.stab.accept_no_sensor")}</span>
              </div>
            </div>
          {:else}
            <p class="small dim">{tk("ui.stab.limit", { c: info.limit_c })}</p>
          {/if}
        {/if}
        {#if error}<StateView kind="error" title={tk("ui.common.error")} text={error.message} />{/if}
        <div class="actions">
          <Button variant="primary" icon="play" disabled={!canStart} onclick={() => (confirming = true)}>{tk("ui.stab.start")}</Button>
        </div>
      </Card>
    {/if}

    {#if records.length}
      <Card title={tk("ui.stab.saved_title")}>
        <ul class="saved">
          {#each records as r (r.id)}
            <li>
              <div>
                <strong class:bad={BAD.has(r.outcome)}>{tk(`ui.stab.outcome.${r.outcome}.title`)}</strong>
                <span class="small muted">
                  · {r.adapter || "—"} · {clock(r.elapsed_s)} · {dateTime(r.started_unix_ms / 1000)}
                  {#if r.bad_cores.length}· {tk("ui.stab.bad_cores", { cores: coreList(r.bad_cores) })}{/if}
                </span>
              </div>
              <Button small variant="secondary" icon="trash" onclick={() => remove(r.id)}>{tk("ui.stab.delete")}</Button>
            </li>
          {/each}
        </ul>
      </Card>
    {/if}
  </div>
</Page>

{#if confirming}
  <Modal
    title={tk(`ui.stab.confirm.${kind}.title`)}
    confirmLabel={tk("ui.stab.confirm_ok")}
    onconfirm={start}
    oncancel={() => (confirming = false)}
  >
    <p>{tk(`ui.stab.confirm.${kind}.text`, { min: seconds / 60 })}</p>
  </Modal>
{/if}

<style>
  .stack {
    display: grid;
    gap: calc(var(--space) * 4);
  }
  .now {
    display: grid;
    gap: calc(var(--space) * 2);
    margin-bottom: calc(var(--space) * 4);
  }
  .time {
    font-size: 20px;
  }
  .progress {
    height: 6px;
    overflow: hidden;
    border-radius: 3px;
    background: var(--bg-input);
  }
  .progress i {
    display: block;
    height: 100%;
    background: linear-gradient(90deg, var(--accent-from), var(--accent-to));
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
    gap: calc(var(--space) * 4);
    margin: 0 0 calc(var(--space) * 3);
  }
  dt {
    margin-bottom: var(--space);
  }
  dd {
    margin: 0;
  }
  .duration {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: calc(var(--space) * 3);
    margin-top: calc(var(--space) * 3);
  }
  .row {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
    margin-top: calc(var(--space) * 3);
  }
  .result {
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    margin-bottom: calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }
  .ok-box {
    border-color: var(--ok);
  }
  .bad-box {
    border-color: var(--danger);
  }
  .box-warn {
    display: grid;
    gap: calc(var(--space) * 2);
    padding: calc(var(--space) * 3);
    margin-top: calc(var(--space) * 3);
    border: 1px solid var(--warn);
    border-radius: var(--radius-md);
  }
  .notes,
  .saved {
    display: grid;
    gap: calc(var(--space) * 2);
    padding: 0;
    margin: 0;
    list-style: none;
  }
  .saved li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
  }
  .bad {
    color: var(--danger);
  }
  .warn {
    color: var(--warn);
  }
  p {
    margin: calc(var(--space) * 2) 0 0;
  }
  .result p,
  .now p {
    margin: 0;
  }
</style>
