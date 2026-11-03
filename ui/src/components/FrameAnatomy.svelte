<script lang="ts">
  import type { FrameAnatomy } from "../lib/bindings/FrameAnatomy";
  import type { SpikeAnatomy } from "../lib/bindings/SpikeAnatomy";
  import type { Stall } from "../lib/bindings/Stall";
  import type { ThreadState } from "../lib/bindings/ThreadState";
  import { tk } from "../lib/i18n";

  // Анатомія кадру за стовпцями PresentMon 2.x: типовий кадр (CPU готує / чекає; GPU
  // починає / працює / простоює; до показу) і що розтягнуло найгірші ривки. Числа й
  // класифікацію рахує бекенд; тут лише показ.

  let { anatomy, start = 0 }: { anatomy: FrameAnatomy; start?: number } = $props();

  /** Скільки ривків показувати в таблиці. */
  const SHOWN = 8;
  const STALLS: Stall[] = ["cpu_busy", "wait_gpu", "wait"];
  /** Порядок як у `main_counts` бекенду. */
  const STATES: ThreadState[] = ["running", "ready", "sync", "page_in", "sleep", "other"];

  /** Скільки мс головний потік був у стані, що переважив (бекенд уже вибрав стан). */
  function stateMs(s: SpikeAnatomy): number | null {
    const m = s.main;
    if (!m || !s.main_state) return null;
    const by: Record<ThreadState, number> = {
      running: m.running,
      ready: m.ready,
      sync: m.sync,
      page_in: m.page_in,
      sleep: m.sleep,
      other: m.other,
    };
    return by[s.main_state];
  }
  const traced = $derived(anatomy.main_counts.some((n) => n > 0));

  const ms = (v: number | null | undefined) => (v == null ? "—" : v.toFixed(1));
  const ty = $derived(anatomy.typical);
  /** Шкала смуг: найдовша з двох доріжок типового кадру. */
  const scale = $derived(
    Math.max(
      ty.frame_ms,
      (ty.cpu_busy ?? 0) + (ty.cpu_wait ?? 0),
      (ty.gpu_latency ?? 0) + (ty.gpu_busy ?? 0) + (ty.gpu_wait ?? 0),
      0.001,
    ),
  );
  const w = (v: number | null | undefined) => `${(((v ?? 0) / scale) * 100).toFixed(2)}%`;
  const queued = $derived(anatomy.queued_share != null && anatomy.queued_share >= 0.1);
</script>

<div class="anatomy">
  <p class="small muted">{tk("ui.anatomy.typical", { ms: ms(ty.frame_ms) })}</p>
  <div class="lanes" role="img" aria-label={tk("ui.anatomy.typical", { ms: ms(ty.frame_ms) })}>
    <span class="lane-name small">CPU</span>
    <div class="lane">
      <i class="seg cpu" style:width={w(ty.cpu_busy)} title={tk("ui.anatomy.cpu_busy")}></i>
      <i class="seg wait" style:width={w(ty.cpu_wait)} title={tk("ui.anatomy.cpu_wait")}></i>
    </div>
    <span class="lane-name small">GPU</span>
    <div class="lane">
      <i class="seg gap" style:width={w(ty.gpu_latency)} title={tk("ui.anatomy.gpu_latency")}></i>
      <i class="seg gpu" style:width={w(ty.gpu_busy)} title={tk("ui.anatomy.gpu_busy")}></i>
      <i class="seg wait" style:width={w(ty.gpu_wait)} title={tk("ui.anatomy.gpu_wait")}></i>
    </div>
  </div>
  <dl class="parts small">
    <dt><i class="dot cpu"></i>{tk("ui.anatomy.cpu_busy")}</dt>
    <dd class="mono">{ms(ty.cpu_busy)} {tk("ui.anatomy.ms")}</dd>
    <dt><i class="dot wait"></i>{tk("ui.anatomy.cpu_wait")}</dt>
    <dd class="mono">{ms(ty.cpu_wait)} {tk("ui.anatomy.ms")}</dd>
    <dt><i class="dot gap"></i>{tk("ui.anatomy.gpu_latency")}</dt>
    <dd class="mono">{ms(ty.gpu_latency)} {tk("ui.anatomy.ms")}</dd>
    <dt><i class="dot gpu"></i>{tk("ui.anatomy.gpu_busy")}</dt>
    <dd class="mono">{ms(ty.gpu_busy)} {tk("ui.anatomy.ms")}</dd>
    <dt><i class="dot wait"></i>{tk("ui.anatomy.gpu_wait")}</dt>
    <dd class="mono">{ms(ty.gpu_wait)} {tk("ui.anatomy.ms")}</dd>
    <dt><i class="dot"></i>{tk("ui.anatomy.until_displayed")}</dt>
    <dd class="mono">{ms(ty.until_displayed)} {tk("ui.anatomy.ms")}</dd>
  </dl>
  {#if queued}
    <p class="small warn">{tk("ui.anatomy.queued", { pct: Math.round((anatomy.queued_share ?? 0) * 100) })}</p>
  {/if}

  {#if anatomy.spikes.length}
    <h3 class="small muted">{tk("ui.anatomy.spikes_title", { n: anatomy.spikes.length })}</h3>
    <div class="chips">
      {#each STALLS as s, i (s)}
        {#if anatomy.stall_counts[i]}
          <span class="chip small {s}">{tk(`ui.anatomy.stall.${s}`)} · {anatomy.stall_counts[i]}</span>
        {/if}
      {/each}
    </div>
    {#if traced}
      <p class="small muted">{tk("ui.anatomy.main_title")}</p>
      <div class="chips">
        {#each STATES as st, i (st)}
          {#if anatomy.main_counts[i]}
            <span class="chip small">{tk(`ui.anatomy.main.${st}`)} · {anatomy.main_counts[i]}</span>
          {/if}
        {/each}
      </div>
    {/if}
    <table class="spikes small">
      <thead>
        <tr>
          <th>{tk("ui.anatomy.col.time")}</th>
          <th>{tk("ui.anatomy.col.frame")}</th>
          <th>{tk("ui.anatomy.col.cpu_busy")}</th>
          <th>{tk("ui.anatomy.col.cpu_wait")}</th>
          <th>{tk("ui.anatomy.col.gpu")}</th>
          <th>{tk("ui.anatomy.col.stall")}</th>
          {#if traced}<th>{tk("ui.anatomy.col.main")}</th>{/if}
        </tr>
      </thead>
      <tbody>
        {#each anatomy.spikes.slice(0, SHOWN) as s (s.t_ms)}
          <tr>
            <td class="mono">{(Math.max(0, s.t_ms - start) / 1000).toFixed(1)} с</td>
            <td class="mono">{ms(s.parts.frame_ms)}</td>
            <td class="mono">{ms(s.parts.cpu_busy)}</td>
            <td class="mono">{ms(s.parts.cpu_wait)}</td>
            <td class="mono">{ms(s.parts.gpu_busy)} / {ms(s.prev_gpu_busy)}</td>
            <td>{s.stall ? tk(`ui.anatomy.stall.${s.stall}`) : "—"}</td>
            {#if traced}
              <td
                title={s.main
                  ? tk("ui.anatomy.main_detail", {
                      run: ms(s.main.running),
                      ready: ms(s.main.ready),
                      sync: ms(s.main.sync),
                      disk: ms(s.main.page_in),
                      sleep: ms(s.main.sleep),
                    })
                  : undefined}
              >
                {s.main_state ? `${tk(`ui.anatomy.main.${s.main_state}`)} ${ms(stateMs(s))} ${tk("ui.anatomy.ms")}` : "—"}
              </td>
            {/if}
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
  <p class="small dim">{tk("ui.anatomy.note")}</p>
</div>

<style>
  .anatomy {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .lanes {
    display: grid;
    grid-template-columns: 40px 1fr;
    align-items: center;
    gap: calc(var(--space) * 2) calc(var(--space) * 3);
  }
  .lane-name {
    color: var(--text-muted);
  }
  .lane {
    display: flex;
    height: 14px;
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    overflow: hidden;
  }
  .seg {
    display: block;
    height: 100%;
  }
  .cpu {
    background: var(--accent-from);
  }
  .gpu {
    background: var(--ok);
  }
  .wait {
    background: var(--border-strong);
  }
  .gap {
    background: transparent;
  }
  .parts {
    display: grid;
    grid-template-columns: max-content max-content;
    gap: calc(var(--space) * 1) calc(var(--space) * 4);
    margin: 0;
  }
  .parts dt {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    color: var(--text-muted);
  }
  .parts dd {
    margin: 0;
    text-align: right;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .dot.gap {
    border: 1px dashed var(--text-dim);
    background: transparent;
  }
  .dot.cpu {
    background: var(--accent-from);
  }
  .dot.gpu {
    background: var(--ok);
  }
  .dot.wait {
    background: var(--border-strong);
  }
  .warn {
    color: var(--warn);
    margin: 0;
  }
  h3 {
    margin: calc(var(--space) * 2) 0 0;
    font-weight: 600;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 2);
  }
  .chip {
    padding: 1px calc(var(--space) * 2);
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
  }
  .chip.cpu_busy {
    border-color: var(--accent-from);
  }
  .chip.wait_gpu {
    border-color: var(--ok);
  }
  .spikes {
    width: 100%;
    border-collapse: collapse;
  }
  .spikes th {
    text-align: left;
    font-weight: 500;
    color: var(--text-muted);
    padding: calc(var(--space) * 1) calc(var(--space) * 2);
    border-bottom: 1px solid var(--border);
  }
  .spikes td {
    padding: calc(var(--space) * 1) calc(var(--space) * 2);
    border-bottom: 1px solid var(--border);
  }
  p {
    margin: 0;
  }
</style>
