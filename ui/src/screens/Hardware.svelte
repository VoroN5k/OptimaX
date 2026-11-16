<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { HardwareInfo } from "../lib/bindings/HardwareInfo";
  import type { Snapshot } from "../lib/bindings/Snapshot";
  import { gib, pct, rate } from "../lib/format";
  import { tk } from "../lib/i18n";
  import Card from "../components/Card.svelte";
  import KpiStrip from "../components/KpiStrip.svelte";
  import Page from "../components/Page.svelte";
  import StateView from "../components/StateView.svelte";

  // «Залізо» (docs/DESIGN.md розд. 6–7): смуга живих показників, процесор з кожним ядром (для ігор
  // важливе найзавантаженіше), відеокарта шкалами, пам'ять з модулями (частота проти номіналу),
  // монітори (частота проти максимуму), найактивніші процеси. Лише показ виміряного; висновки —
  // в аудиті «Головної». Метрики збираються лише поки екран відкритий і вікно видно.
  let info = $state<HardwareInfo | null>(null);
  let snap = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);

  onMount(() => {
    api.hardwareInfo().then(
      (i) => (info = i),
      (e) => (error = asCmdError(e).message),
    );
    const unlisten = on("metrics", (s) => (snap = s));
    const sync = () => (document.hidden ? api.metricsStop() : api.metricsStart(500));
    void sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      document.removeEventListener("visibilitychange", sync);
      void api.metricsStop();
      void unlisten.then((u) => u());
    };
  });

  /** Ядро «в стелі» — як у правилі CPU-bound (CLAUDE.md розд. 9). */
  const CEIL = 95;
  const gpu = $derived(snap?.gpus[0] ?? null);
  const ram = $derived(snap?.memory ?? null);
  const busiest = $derived(snap ? Math.max(...snap.cpu.per_core_percent, 0) : null);
  const avgMhz = $derived.by(() => {
    const m = snap?.cpu.per_core_mhz.filter((v) => v > 0) ?? [];
    return m.length ? Math.round(m.reduce((a, b) => a + b, 0) / m.length) : null;
  });
  const share = (a: number | null | undefined, b: number | null | undefined) =>
    a != null && b ? Math.max(0, Math.min(100, (a / b) * 100)) : null;
  /** Причини обмеження частоти, крім «простою» (це не обмеження під навантаженням). */
  const throttle = $derived((gpu?.throttle ?? []).filter((r) => r !== "idle"));
  const date = (d: { year: number; month: number; day: number } | null | undefined) =>
    d ? `${d.year}-${String(d.month).padStart(2, "0")}-${String(d.day).padStart(2, "0")}` : null;
  const memType = (t: unknown) => (typeof t === "string" ? t.toUpperCase() : "?");
</script>

{#snippet meter(label: string, value: string, fill: number | null, warn = false)}
  <div class="meter">
    <div class="m-top">
      <span class="small muted">{label}</span>
      <span class="mono m-val" class:warn>{value}</span>
    </div>
    <div class="m-track" aria-hidden="true">
      {#if fill != null}<i class="m-fill" class:warn style:transform="scaleX({fill / 100})"></i>{/if}
    </div>
  </div>
{/snippet}

<Page title={tk("ui.hw.title")} subtitle={tk("ui.hw.subtitle")}>
  {#if error}
    <Card><StateView kind="error" title={tk("ui.common.error")} text={error} /></Card>
  {/if}

  <KpiStrip
    label={tk("ui.hw.kpi")}
    items={[
      { label: tk("ui.hw.kpi.cpu"), value: snap ? pct(snap.cpu.total_percent) : null },
      {
        label: tk("ui.hw.kpi.core"),
        value: busiest != null ? pct(busiest) : null,
        tone: busiest != null && busiest >= CEIL ? "warn" : undefined,
      },
      { label: tk("ui.hw.kpi.gpu"), value: gpu?.utilization_percent != null ? pct(gpu.utilization_percent) : null },
      { label: tk("ui.hw.kpi.ram"), value: ram ? `${gib(ram.used_bytes)} / ${gib(ram.total_bytes)}` : null },
    ]}
  >
    {#snippet footer()}
      {#if snap?.cpu.temp_c != null}<span>{tk("ui.hw.cpu_temp", { v: snap.cpu.temp_c.toFixed(0) })}</span>{/if}
      {#if gpu?.temperature_c != null}<span>{tk("ui.hw.gpu_temp", { v: gpu.temperature_c })}</span>{/if}
      {#if snap}<span>{tk("ui.hw.disk", { r: rate(snap.disk.read_bytes_per_sec), w: rate(snap.disk.write_bytes_per_sec) })}</span>{/if}
      {#if !snap}<span>{tk("ui.hw.waiting")}</span>{/if}
    {/snippet}
  </KpiStrip>

  <div class="bento">
    <section class="tile t cpu" aria-labelledby="hw-cpu">
      <header>
        <h2 id="hw-cpu">{tk("ui.hw.cpu")}</h2>
        <p class="small muted">
          {info ? tk("ui.hw.cpu_sub", { name: info.cpu_name, cores: info.physical_cores ?? "?", threads: info.logical_cores }) : tk("ui.common.loading")}
        </p>
      </header>
      {#if snap}
        <!-- Рівна сітка: до 16 ядер у рядку (32 потоки — два рядки по 16), без «хвоста» 14 + 2. -->
        <div
          class="cores"
          role="list"
          aria-label={tk("ui.hw.cores")}
          style:--cols={Math.min(16, Math.ceil(snap.cpu.per_core_percent.length / Math.ceil(snap.cpu.per_core_percent.length / 16)))}
        >
          {#each snap.cpu.per_core_percent as v, i (i)}
            <div class="core" class:hot={v >= CEIL} role="listitem" aria-label="{tk('ui.hw.core', { n: i })}: {pct(v)}">
              <i class="core-fill" style:transform="scaleY({Math.min(100, v) / 100})" aria-hidden="true"></i>
              <span class="core-v mono">{Math.round(v)}</span>
            </div>
          {/each}
        </div>
        <p class="small dim">
          {tk("ui.hw.cores_hint")}
          {#if avgMhz}· {tk("ui.hw.avg_clock", { v: avgMhz })}{/if}
        </p>
      {/if}
      {#if info?.firmware?.bios}
        {@const b = info.firmware.bios}
        <p class="small dim foot">{tk("ui.hw.bios", { vendor: b.vendor, version: b.version, date: date(b.release_date) ?? "—" })}</p>
      {/if}
    </section>

    <section class="tile t gpu" aria-labelledby="hw-gpu">
      <header>
        <h2 id="hw-gpu">{tk("ui.hw.gpu")}</h2>
        <p class="small muted">{gpu?.name ?? info?.gpus[0]?.name ?? "—"}</p>
      </header>
      {#if gpu}
        <div class="meters">
          {@render meter(tk("ui.hw.load"), pct(gpu.utilization_percent), gpu.utilization_percent)}
          {@render meter(
            "VRAM",
            `${gib(gpu.vram_used_bytes)} / ${gib(gpu.vram_total_bytes)}`,
            share(gpu.vram_used_bytes, gpu.vram_total_bytes),
            (share(gpu.vram_used_bytes, gpu.vram_total_bytes) ?? 0) > 90,
          )}
          {@render meter(
            tk("ui.hw.power"),
            `${gpu.power_w?.toFixed(0) ?? "—"} / ${gpu.power_limit_w?.toFixed(0) ?? "—"} Вт`,
            share(gpu.power_w, gpu.power_limit_w),
          )}
        </div>
        <dl class="kv">
          <dt>{tk("ui.hw.clock")}</dt><dd class="mono">{gpu.core_clock_mhz ?? "—"} / {gpu.max_core_clock_mhz ?? "—"} МГц</dd>
          <dt>{tk("ui.hw.temp")}</dt><dd class="mono">{gpu.temperature_c ?? "—"} °C</dd>
          {#if gpu.fan_percent != null}<dt>{tk("ui.hw.fan")}</dt><dd class="mono">{gpu.fan_percent}%</dd>{/if}
          <dt>{tk("ui.hw.throttle")}</dt>
          <dd>
            {#if throttle.length}
              <span class="warn">{throttle.map((r) => tk(`ui.hw.thr.${r}`)).join(" · ")}</span>
            {:else}
              <span class="muted">{tk("ui.hw.thr.none")}</span>
            {/if}
          </dd>
        </dl>
      {:else}
        <p class="small muted">{tk("ui.hw.no_gpu_metrics")}</p>
      {/if}
    </section>

    <section class="tile t half" aria-labelledby="hw-ram">
      <header>
        <h2 id="hw-ram">{tk("ui.hw.ram")}</h2>
        <p class="small muted">
          {info?.firmware ? tk("ui.hw.ram_sub", { n: info.firmware.memory.length, slots: info.firmware.memory_slots }) : ""}
        </p>
      </header>
      {#if ram}
        {@render meter(tk("ui.hw.used"), `${gib(ram.used_bytes)} / ${gib(ram.total_bytes)}`, share(ram.used_bytes, ram.total_bytes))}
      {/if}
      {#if info?.firmware?.memory.length}
        <table class="list">
          <thead>
            <tr><th>{tk("ui.hw.slot")}</th><th>{tk("ui.hw.module")}</th><th class="r">{tk("ui.hw.speed")}</th></tr>
          </thead>
          <tbody>
            {#each info.firmware.memory as m, i (i)}
              {@const slow = m.configured_mts != null && m.rated_mts != null && m.configured_mts < m.rated_mts}
              <tr>
                <td class="mono">{m.locator}</td>
                <td>
                  {Math.round(m.size_mib / 1024)} ГБ {memType(m.memory_type)}
                  <span class="dim small part">{m.part_number}</span>
                </td>
                <td class="r mono" class:warn={slow}>
                  {m.rated_mts ? `${m.configured_mts ?? "?"} / ${m.rated_mts}` : (m.configured_mts ?? "?")} MT/s
                  {#if slow}<span class="small note">{tk("ui.hw.below_rated")}</span>{/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section class="tile t half" aria-labelledby="hw-mon">
      <header>
        <h2 id="hw-mon">{tk("ui.hw.displays")}</h2>
      </header>
      {#if info?.displays.length}
        <table class="list">
          <thead>
            <tr><th>{tk("ui.hw.monitor")}</th><th>{tk("ui.hw.resolution")}</th><th class="r">{tk("ui.hw.refresh")}</th></tr>
          </thead>
          <tbody>
            {#each info.displays as d (d.device_name)}
              {@const low = d.current_hz < d.max_hz_at_current_res}
              <tr>
                <td>{d.monitor_name}{#if d.primary}<span class="chip small">{tk("ui.hw.primary")}</span>{/if}</td>
                <td class="mono">{d.width}×{d.height}</td>
                <td class="r mono" class:warn={low}>
                  {d.current_hz} / {d.max_hz_at_current_res} Гц
                  {#if low}<span class="small note">{tk("ui.hw.hz_below")}</span>{/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <p class="small muted">{info ? "—" : tk("ui.common.loading")}</p>
      {/if}
    </section>
  </div>

  {#if snap?.top_processes.length}
    {@const top = Math.max(...snap.top_processes.map((p) => p.cpu_percent), 1)}
    <section class="tile t" aria-labelledby="hw-top">
      <header><h2 id="hw-top">{tk("ui.hw.top")}</h2></header>
      <table class="list procs">
        <thead><tr><th>{tk("ui.hw.process")}</th><th>CPU</th><th class="r">{tk("ui.hw.memory")}</th></tr></thead>
        <tbody>
          {#each snap.top_processes as p (p.pid)}
            <tr>
              <td class="mono">{p.name}</td>
              <td>
                <span class="pcell">
                  <span class="pbar"><i style:transform="scaleX({p.cpu_percent / top})" aria-hidden="true"></i></span>
                  <span class="mono">{pct(p.cpu_percent, 1)}</span>
                </span>
              </td>
              <td class="r mono">{gib(p.memory_bytes)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}
</Page>

<style>
  .bento {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: var(--gap);
  }
  .t {
    padding: calc(var(--space) * 6) calc(var(--space) * 7);
    display: grid;
    gap: calc(var(--space) * 4);
    align-content: start;
    min-width: 0;
  }
  .cpu {
    grid-column: span 7;
  }
  .gpu {
    grid-column: span 5;
  }
  .half {
    grid-column: span 6;
  }
  @media (max-width: 1100px) {
    .cpu,
    .gpu,
    .half {
      grid-column: 1 / -1;
    }
  }
  header {
    display: grid;
    gap: 2px;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-tile);
  }
  p {
    margin: 0;
  }
  /* Ядра: плитка на кожне, заповнення знизу, число зверху; «в стелі» — помаранчевим і числом. */
  .cores {
    display: grid;
    grid-template-columns: repeat(var(--cols, 8), minmax(0, 1fr));
    gap: 5px;
  }
  .core {
    position: relative;
    height: 96px;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    overflow: hidden;
  }
  .core-fill {
    position: absolute;
    inset: 0;
    transform-origin: bottom;
    background: linear-gradient(180deg, var(--a1), var(--a2));
    opacity: 0.55;
    transition: transform var(--dur-slow) var(--ease);
  }
  .core.hot .core-fill {
    background: var(--warn-gradient);
    opacity: 0.9;
  }
  .core-v {
    position: absolute;
    top: 4px;
    left: 0;
    right: 0;
    text-align: center;
    font-size: 11px;
    color: var(--text);
  }
  .foot {
    padding-top: calc(var(--space) * 3);
    border-top: 1px solid var(--line-soft);
  }
  .meters {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .meter {
    display: grid;
    gap: 6px;
  }
  .m-top {
    display: flex;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
  }
  .m-val {
    font-size: 13px;
  }
  .m-track {
    height: 6px;
    border-radius: 999px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .m-fill {
    display: block;
    height: 100%;
    transform-origin: left;
    background: var(--gpu);
    transition: transform var(--dur-slow) var(--ease);
  }
  .half .m-fill {
    background: linear-gradient(90deg, var(--a2), var(--a1));
  }
  .m-fill.warn {
    background: var(--warn-gradient);
  }
  .kv {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 1.5) calc(var(--space) * 4);
    margin: 0;
    font-size: 13px;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
  }
  .warn {
    color: var(--warn);
  }
  .list {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .list th {
    text-align: left;
    color: var(--text-3);
    font-weight: 500;
    font-size: 11.5px;
    padding-bottom: calc(var(--space) * 2);
  }
  .list td {
    padding: calc(var(--space) * 2) 0;
    border-top: 1px solid var(--line-soft);
    vertical-align: top;
  }
  .list td + td,
  .list th + th {
    padding-left: calc(var(--space) * 4);
  }
  .r {
    text-align: right;
    white-space: nowrap;
  }
  .part {
    display: block;
    font-family: var(--font-mono);
  }
  .note {
    display: block;
    font-family: var(--font-ui, inherit);
  }
  .chip {
    margin-left: calc(var(--space) * 2);
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--fill-2);
    color: var(--text-2);
  }
  .pcell {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
  .pbar {
    flex: 0 0 120px;
    height: 4px;
    border-radius: 999px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .pbar i {
    display: block;
    height: 100%;
    transform-origin: left;
    background: linear-gradient(90deg, var(--a2), var(--a1));
  }
</style>
