<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { HardwareInfo } from "../lib/bindings/HardwareInfo";
  import type { Snapshot } from "../lib/bindings/Snapshot";
  import { gib, mib, pct, rate } from "../lib/format";
  import { tk } from "../lib/i18n";
  import Card from "../components/Card.svelte";
  import Page from "../components/Page.svelte";
  import StateView from "../components/StateView.svelte";

  let info = $state<HardwareInfo | null>(null);
  let snap = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);

  onMount(() => {
    api.hardwareInfo().then(
      (i) => (info = i),
      (e) => (error = asCmdError(e).message),
    );
    const unlisten = on("metrics", (s) => (snap = s));
    // Метрики лише поки екран відкритий і вікно видно — щоб не навантажувати систему даремно.
    const sync = () => (document.hidden ? api.metricsStop() : api.metricsStart(500));
    void sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      document.removeEventListener("visibilitychange", sync);
      void api.metricsStop();
      void unlisten.then((u) => u());
    };
  });

  const gpu = $derived(snap?.gpus[0] ?? null);
  const ram = $derived(snap?.memory ?? null);
  const busiest = $derived(snap ? Math.max(...snap.cpu.per_core_percent, 0) : 0);
</script>

<Page title={tk("ui.hw.title")} subtitle={tk("ui.hw.subtitle")}>
  {#if error}
    <Card><StateView kind="error" title={tk("ui.common.error")} text={error} /></Card>
  {/if}

  <div class="grid">
    <Card title={tk("ui.hw.cpu")} subtitle={info ? `${info.cpu_name} · ${info.physical_cores ?? "?"}/${info.logical_cores}` : tk("ui.common.loading")}>
      {#if snap}
        <div class="big">
          <span>{pct(snap.cpu.total_percent)}</span>
          <span class="small muted">{tk("ui.hw.busiest", { v: pct(busiest) })}</span>
        </div>
        <div class="cores" aria-label={tk("ui.hw.cores")}>
          {#each snap.cpu.per_core_percent as v, i (i)}
            <div class="core" title="{tk('ui.hw.core', { n: i })}: {pct(v)}">
              <div class="core-fill" class:hot={v >= 95} style:transform="scaleY({Math.min(100, v) / 100})"></div>
            </div>
          {/each}
        </div>
        <p class="small dim">{tk("ui.hw.cores_hint")}</p>
      {/if}
    </Card>

    <Card title={tk("ui.hw.gpu")} subtitle={gpu?.name ?? info?.gpus[0]?.name ?? "—"}>
      {#if gpu}
        <dl class="kv">
          <dt>{tk("ui.hw.load")}</dt><dd>{pct(gpu.utilization_percent)}</dd>
          <dt>VRAM</dt><dd>{mib(gpu.vram_used_bytes)} / {mib(gpu.vram_total_bytes)}</dd>
          <dt>{tk("ui.hw.clock")}</dt><dd>{gpu.core_clock_mhz ?? "—"} / {gpu.max_core_clock_mhz ?? "—"} МГц</dd>
          <dt>{tk("ui.hw.temp")}</dt><dd>{gpu.temperature_c ?? "—"} °C</dd>
          <dt>{tk("ui.hw.power")}</dt><dd>{gpu.power_w?.toFixed(0) ?? "—"} / {gpu.power_limit_w?.toFixed(0) ?? "—"} Вт</dd>
          <dt>{tk("ui.hw.throttle")}</dt><dd class="mono small">{gpu.throttle.join(", ") || "—"}</dd>
        </dl>
      {:else}
        <p class="small muted">{tk("ui.hw.no_gpu_metrics")}</p>
      {/if}
    </Card>

    <Card title={tk("ui.hw.ram")} subtitle={ram ? `${gib(ram.used_bytes)} / ${gib(ram.total_bytes)}` : ""}>
      {#if info?.firmware}
        <ul class="modules">
          {#each info.firmware.memory as m, i (i)}
            <li>
              <span class="mono">{m.locator}</span>
              <span>{Math.round(m.size_mib / 1024)} ГБ {typeof m.memory_type === "string" ? m.memory_type.toUpperCase() : "?"}</span>
              <span class="muted">{m.configured_mts ?? "?"} MT/s</span>
              <span class="dim small">{m.part_number}</span>
            </li>
          {/each}
        </ul>
      {/if}
      {#if snap}
        <p class="small muted">{tk("ui.hw.disk", { r: rate(snap.disk.read_bytes_per_sec), w: rate(snap.disk.write_bytes_per_sec) })}</p>
      {/if}
    </Card>

    <Card title={tk("ui.hw.displays")}>
      <ul class="modules">
        {#each info?.displays ?? [] as d (d.device_name)}
          <li>
            <span>{d.monitor_name}</span>
            <span class="mono">{d.width}×{d.height}</span>
            <span class:warn={d.current_hz < d.max_hz_at_current_res}>{d.current_hz} / {d.max_hz_at_current_res} Гц</span>
          </li>
        {/each}
      </ul>
      {#if info?.firmware?.bios}
        <p class="small muted">BIOS: {info.firmware.bios.vendor} {info.firmware.bios.version}</p>
      {/if}
    </Card>
  </div>

  {#if snap?.top_processes.length}
    <Card title={tk("ui.hw.top")}>
      <table>
        <thead><tr><th>{tk("ui.hw.process")}</th><th>CPU</th><th>{tk("ui.hw.memory")}</th></tr></thead>
        <tbody>
          {#each snap.top_processes as p (p.pid)}
            <tr><td class="mono">{p.name}</td><td>{pct(p.cpu_percent, 1)}</td><td>{mib(p.memory_bytes)}</td></tr>
          {/each}
        </tbody>
      </table>
    </Card>
  {/if}
</Page>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(380px, 1fr));
    gap: calc(var(--space) * 5);
  }
  .big {
    display: flex;
    align-items: baseline;
    gap: calc(var(--space) * 3);
    margin-bottom: calc(var(--space) * 3);
  }
  .big span:first-child {
    font-size: 28px;
    font-weight: 600;
  }
  .cores {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14px, 1fr));
    gap: 4px;
    height: 64px;
    margin-bottom: calc(var(--space) * 2);
  }
  .core {
    position: relative;
    border-radius: 4px;
    background: var(--bg-input);
    overflow: hidden;
  }
  .core-fill {
    position: absolute;
    inset: 0;
    transform-origin: bottom;
    background: var(--accent-gradient);
    transition: transform var(--dur-slow) var(--ease);
  }
  .core-fill.hot {
    background: var(--danger);
  }
  .kv {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 1.5) calc(var(--space) * 4);
    margin: 0;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
  }
  .modules {
    list-style: none;
    margin: 0 0 calc(var(--space) * 3);
    padding: 0;
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .modules li {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 3);
    align-items: baseline;
  }
  .warn {
    color: var(--warn);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th {
    text-align: left;
    color: var(--text-muted);
    font-weight: 500;
    padding-bottom: calc(var(--space) * 2);
  }
  td {
    padding: calc(var(--space) * 1.5) 0;
    border-top: 1px solid var(--border);
  }
</style>
