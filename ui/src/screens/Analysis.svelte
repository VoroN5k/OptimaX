<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { ConfigSummary } from "../lib/bindings/ConfigSummary";
  import type { DiskSummary } from "../lib/bindings/DiskSummary";
  import type { Finding } from "../lib/bindings/Finding";
  import type { LastGame } from "../lib/bindings/LastGame";
  import type { UiDesired } from "../lib/bindings/UiDesired";
  import { ago, size } from "../lib/format";
  import { t, tk } from "../lib/i18n";
  import { flashBg, ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import FindingCard from "../components/FindingCard.svelte";
  import Page from "../components/Page.svelte";
  import Ring from "../components/Ring.svelte";
  import StateView from "../components/StateView.svelte";
  import TweakApplyModal from "../components/TweakApplyModal.svelte";

  // Головний екран «Стан ПК»: чотири підсумки, обчислені бекендом (останній діагноз гри,
  // аудит конфігурації, швидкий скан диска за правилами, вік драйвера). Під ними — повний
  // список перевірок конфігурації.

  const BALANCED = "381b4222-f694-41f0-9685-ff5bb260df2e";

  type Load<T> = { data: T | null; loading: boolean; error: string | null };
  let game = $state<Load<LastGame>>({ data: null, loading: true, error: null });
  let config = $state<Load<ConfigSummary>>({ data: null, loading: true, error: null });
  let disk = $state<Load<DiskSummary>>({ data: null, loading: true, error: null });
  let fix = $state<UiDesired | null>(null);
  let configEl = $state<HTMLElement | null>(null);

  async function load<T>(slot: Load<T>, f: () => Promise<T>) {
    slot.loading = true;
    slot.error = null;
    try {
      slot.data = await f();
    } catch (e) {
      slot.error = asCmdError(e).message;
    } finally {
      slot.loading = false;
    }
  }

  function refresh(force: boolean) {
    ui.bgMode = "scanning";
    void Promise.all([
      load(game, () => api.dashboardLastGame()),
      load(config, () => api.dashboardConfig(force)),
      load(disk, () => api.dashboardDisk(force)),
    ]).then(() => {
      const problems = (config.data?.high ?? 0) + (config.data?.moderate ?? 0);
      flashBg(problems > 0 ? "warn" : "ok");
    });
  }

  onMount(() => refresh(false));

  /** Знахідки, які OptimaX уміє виправити сам (через перегляд змін і журнал). */
  function fixFor(f: Finding): UiDesired | null {
    if (f.id === "power.saver") return { tweak: "power_scheme", value: BALANCED };
    if (f.id === "os.game_mode_off") return { tweak: "game_mode", value: true };
    return null;
  }

  const report = $derived(config.data?.report ?? null);
  const problems = $derived((config.data?.high ?? 0) + (config.data?.moderate ?? 0));
  const counts = $derived({
    high: config.data?.high ?? 0,
    moderate: config.data?.moderate ?? 0,
    info: report?.findings.filter((f) => f.impact === "info").length ?? 0,
  });
  const okShare = $derived(report && report.findings.length ? (counts.info / report.findings.length) * 100 : 0);
  /** Аудит уже впорядкував знахідки від найвпливовіших. */
  const findings = $derived(report?.findings ?? []);

  const fps = (v: number | null) => (v == null ? "—" : v.toFixed(0));
  /** Менше за це — «звільняти майже нічого» (а не «0.1 ГБ можна звільнити»). */
  const LITTLE = 256 * 1048576;

  function driverTone(f: Finding | null): "ok" | "warn" | "dim" {
    if (!f) return "dim";
    return f.id === "driver.ok" ? "ok" : "warn";
  }

  function openConfig() {
    configEl?.scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }
</script>

{#snippet tile(
  tone: "ok" | "warn" | "danger" | "info" | "dim",
  label: string,
  value: string,
  sub: string,
  loading: boolean,
  onclick: () => void,
)}
  <button type="button" class="tile {tone}" {onclick} aria-busy={loading}>
    <span class="label small muted"><i class="dot" aria-hidden="true"></i>{label}</span>
    {#if loading}
      <span class="value loading"><span class="spinner" aria-hidden="true"></span>{tk("ui.home.checking")}</span>
    {:else}
      <span class="value">{value}</span>
      <span class="sub small muted">{sub}</span>
    {/if}
  </button>
{/snippet}

<Page title={tk("ui.home.title")} subtitle={tk("ui.home.subtitle")}>
  <Card title={tk("ui.home.state")}>
    <div class="tiles">
      {#if game.data}
        {@render tile(
          game.data.bottleneck === "unclear" ? "dim" : "info",
          tk("ui.home.game"),
          t(game.data.title),
          tk("ui.home.game_sub", {
            avg: fps(game.data.avg_fps),
            low: fps(game.data.low_1pct_fps),
            ago: ago(game.data.at_unix, tk),
          }),
          false,
          () => (ui.screen = "diagnosis"),
        )}
      {:else}
        {@render tile("dim", tk("ui.home.game"), tk("ui.home.game_none"), tk("ui.home.game_none_sub"), game.loading, () => (ui.screen = "diagnosis"))}
      {/if}

      {#if config.error}
        {@render tile("dim", tk("ui.home.config"), tk("ui.home.failed"), config.error, false, () => refresh(true))}
      {:else}
        {@render tile(
          counts.high > 0 ? "danger" : problems > 0 ? "warn" : "ok",
          tk("ui.home.config"),
          problems > 0 ? tk("ui.home.config_n", { n: problems }) : tk("ui.home.config_ok"),
          problems > 0 ? tk("ui.home.config_sub", { high: counts.high, moderate: counts.moderate }) : tk("ui.home.config_ok_sub"),
          config.loading,
          openConfig,
        )}
      {/if}

      {#if disk.error}
        {@render tile("dim", tk("ui.home.disk"), tk("ui.home.failed"), disk.error, false, () => (ui.screen = "cleanup"))}
      {:else if disk.data}
        {@render tile(
          disk.data.safe_bytes >= LITTLE ? "info" : "ok",
          tk("ui.home.disk_label", { drive: disk.data.drive.replace(/\\$/, "") }),
          disk.data.safe_bytes >= LITTLE ? tk("ui.home.disk_free", { size: size(disk.data.safe_bytes) }) : tk("ui.home.disk_little"),
          disk.data.free_bytes != null && disk.data.total_bytes != null
            ? tk("ui.home.disk_sub", {
                more: size(disk.data.consequences_bytes),
                free: size(disk.data.free_bytes),
                total: size(disk.data.total_bytes),
              })
            : tk("ui.home.disk_sub_short", { more: size(disk.data.consequences_bytes) }),
          disk.loading,
          () => (ui.screen = "cleanup"),
        )}
      {:else}
        {@render tile("dim", tk("ui.home.disk"), "", "", true, () => (ui.screen = "cleanup"))}
      {/if}

      {#if config.data?.driver}
        {@render tile(
          driverTone(config.data.driver),
          tk("ui.home.driver"),
          t(config.data.driver.title),
          t(config.data.driver.detail),
          config.loading,
          () => (ui.screen = "hardware"),
        )}
      {:else}
        {@render tile("dim", tk("ui.home.driver"), tk("ui.home.driver_none"), tk("ui.home.driver_none_sub"), config.loading, () => (ui.screen = "hardware"))}
      {/if}
    </div>
  </Card>

  <section class="config" bind:this={configEl} aria-label={tk("ui.analysis.title")}>
    <h2 class="section muted">{tk("ui.home.config_section")}</h2>
    {#if config.loading && !report}
      <Card><StateView kind="loading" title={tk("ui.analysis.loading")} /></Card>
    {:else if config.error && !report}
      <Card>
        <StateView kind="error" title={tk("ui.common.error")} text={config.error}>
          {#snippet action()}<Button onclick={() => refresh(true)}>{tk("ui.common.retry")}</Button>{/snippet}
        </StateView>
      </Card>
    {/if}

    {#if report}
      <Card>
        <div class="summary">
          <Ring value={okShare} label={tk("ui.analysis.ring_label")} caption={tk("ui.analysis.ring_caption")} />
          <div class="counts">
            <div class="count high"><strong>{counts.high}</strong><span>{tk("ui.badge.high")}</span></div>
            <div class="count moderate"><strong>{counts.moderate}</strong><span>{tk("ui.badge.moderate")}</span></div>
            <div class="count info"><strong>{counts.info}</strong><span>{tk("ui.badge.info")}</span></div>
          </div>
          <p class="verdict">
            {problems === 0 ? tk("ui.analysis.nothing_to_fix") : tk("ui.analysis.has_findings")}
          </p>
        </div>
      </Card>

      <div class="list">
        {#each findings as f, i (f.id + t(f.title))}
          {@const desired = fixFor(f)}
          <FindingCard finding={f} collapsible expanded={i === 0 && f.impact !== "info"}>
            {#snippet action()}
              {#if desired}
                <Button small variant="primary" icon="check" onclick={() => (fix = desired)}>{tk("ui.analysis.fix")}</Button>
              {/if}
            {/snippet}
          </FindingCard>
        {/each}
      </div>

      {#if report.skipped.length}
        <Card title={tk("ui.analysis.skipped")}>
          <ul class="skipped">
            {#each report.skipped as s (s.check)}
              <li><strong>{t({ key: `audit.check.${s.check}`, args: {} })}</strong> — <span class="muted">{s.error}</span></li>
            {/each}
          </ul>
        </Card>
      {/if}
    {/if}
  </section>

  {#snippet primary()}
    <Button
      variant="primary"
      icon="refresh"
      loading={config.loading || disk.loading}
      disabled={config.loading || disk.loading}
      onclick={() => refresh(true)}>{tk("ui.home.refresh")}</Button
    >
  {/snippet}
</Page>

{#if fix}
  <TweakApplyModal
    desired={fix}
    onclose={(applied) => {
      fix = null;
      if (applied) void load(config, () => api.dashboardConfig(true));
    }}
  />
{/if}

<style>
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: calc(var(--space) * 3);
  }
  .tile {
    display: grid;
    align-content: start;
    gap: calc(var(--space) * 1);
    min-height: 112px;
    padding: calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color var(--dur) var(--ease),
      background-color var(--dur) var(--ease);
  }
  .tile:hover {
    border-color: var(--border-strong);
    background: var(--bg-card-hover);
  }
  .tile:active {
    transform: scale(0.98);
  }
  .tile:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
  }
  .label {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  .ok .dot {
    background: var(--ok);
  }
  .warn .dot {
    background: var(--warn);
  }
  .danger .dot {
    background: var(--danger);
  }
  .info .dot {
    background: var(--info);
  }
  .value {
    font-size: 15px;
    font-weight: 600;
    line-height: 1.35;
  }
  .value.loading {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    color: var(--text-muted);
    font-weight: 400;
  }
  .sub {
    line-height: 1.4;
  }
  .spinner {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-right-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
    .tile:active {
      transform: none;
    }
  }
  .config {
    display: grid;
    gap: calc(var(--space) * 3);
    margin-top: calc(var(--space) * 4);
    scroll-margin-top: calc(var(--space) * 4);
  }
  .section {
    margin: 0;
    padding: 0 calc(var(--space) * 1);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: 11px;
  }
  .summary {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 8);
    flex-wrap: wrap;
  }
  .counts {
    display: flex;
    gap: calc(var(--space) * 3);
  }
  .count {
    display: grid;
    gap: 2px;
    min-width: 108px;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-input);
  }
  .count strong {
    font-size: 24px;
    font-weight: 600;
  }
  .count span {
    font-size: 12px;
    color: var(--text-muted);
  }
  .count.high strong {
    color: var(--danger);
  }
  .count.moderate strong {
    color: var(--warn);
  }
  .count.info strong {
    color: var(--info);
  }
  .verdict {
    flex: 1;
    min-width: 220px;
    color: var(--text-muted);
  }
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .skipped {
    margin: 0;
    padding-left: calc(var(--space) * 5);
    display: grid;
    gap: calc(var(--space) * 1);
  }
</style>
