<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { GpuPref } from "../lib/bindings/GpuPref";
  import type { LaptopView } from "../lib/bindings/LaptopView";
  import { tk } from "../lib/i18n";
  import { logLocal } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import FindingCard from "../components/FindingCard.svelte";
  import Page from "../components/Page.svelte";
  import Segmented from "../components/Segmented.svelte";
  import StateView from "../components/StateView.svelte";

  // ── Overlay GUID constants (mirror overlay_guids in core/power.rs) ──
  const ECONOMY_GUID = "961cc777-2547-4f9d-8174-7d86181b8a7a";
  const BETTER_GUID = "3af9b8d9-7c97-431d-ad78-34a8bfea439f";
  const BEST_GUID = "ded574b5-45a0-4f42-8737-46345c09c238";

  type OverlayMode = "balanced" | "economy" | "better" | "best";

  function guidToMode(guid: string | null): OverlayMode {
    if (!guid) return "balanced";
    const g = guid.toLowerCase();
    if (g === ECONOMY_GUID) return "economy";
    if (g === BETTER_GUID) return "better";
    if (g === BEST_GUID) return "best";
    return "balanced";
  }

  function modeToGuid(mode: OverlayMode): string | null {
    switch (mode) {
      case "economy": return ECONOMY_GUID;
      case "better": return BETTER_GUID;
      case "best": return BEST_GUID;
      default: return null;
    }
  }

  let view = $state<LaptopView | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  let overlayMode = $state<OverlayMode>("balanced");
  let applyingOverlay = $state(false);

  let gpuExe = $state("");
  let gpuPref = $state<GpuPref>("default");
  let applyingGpu = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      view = await api.laptopInfo();
      overlayMode = guidToMode(view.overlay_guid);
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }

  onMount(() => void load());

  async function applyOverlay() {
    if (!view?.overlay_supported) return;
    applyingOverlay = true;
    error = null;
    try {
      const guid = modeToGuid(overlayMode);
      const plan = await api.tweakPlan({ tweak: "power_mode", value: guid });
      if (plan.plan.changes.length === 0) {
        logLocal("info", tk("ui.laptop.power_mode.balanced"));
        return;
      }
      await api.tweakApply(plan.id, false);
      logLocal("info", tk("ui.laptop.power_mode.apply"));
      view = await api.laptopInfo();
      overlayMode = guidToMode(view.overlay_guid);
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      applyingOverlay = false;
    }
  }

  async function applyGpu() {
    const exe = gpuExe.trim();
    if (!exe) return;
    applyingGpu = true;
    error = null;
    try {
      const plan = await api.tweakPlan({
        tweak: "gpu_preference",
        value: { exe_path: exe, preference: gpuPref },
      });
      if (plan.plan.changes.length === 0) {
        logLocal("info", tk("ui.laptop.gpu.apply"));
        return;
      }
      await api.tweakApply(plan.id, false);
      logLocal("info", tk("ui.laptop.gpu.apply"));
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      applyingGpu = false;
    }
  }

  const wearPct = $derived(
    view?.battery?.design_mwh && view.battery.full_charge_mwh
      ? Math.round((1 - view.battery.full_charge_mwh / view.battery.design_mwh) * 100)
      : null,
  );
  const mwh = (v: number | null | undefined) =>
    v != null ? tk("ui.laptop.battery.mwh", { v: (v / 1000).toFixed(1) }) : "—";
  const sourceLabel = (s: string) => tk(`ui.laptop.battery.source.${s}`);
</script>

<Page title={tk("ui.laptop.title")} subtitle={tk("ui.laptop.subtitle")}>
  {#if loading}
    <Card><StateView kind="loading" title={tk("ui.laptop.loading")} /></Card>
  {:else if !view}
    <Card><StateView kind="error" title={tk("ui.common.error")} text={error ?? ""} /></Card>
  {:else}
    <!-- ── Батарея ── -->
    {#if view.is_laptop && view.battery}
      {@const b = view.battery}
      <Card title={tk("ui.laptop.battery.title")}>
        <dl class="kv">
          <dt>{tk("ui.laptop.battery.source")}</dt>
          <dd class:warn={b.source === "dc"}>{sourceLabel(b.source)}</dd>
          <dt>{tk("ui.laptop.battery.charge")}</dt>
          <dd class="mono">{b.charge_pct != null ? `${b.charge_pct}%` : "—"}</dd>
          {#if b.rate_mw != null}
            <dt>{tk("ui.laptop.battery.rate")}</dt>
            <dd class="mono">
              {(Math.abs(b.rate_mw) / 1000).toFixed(1)}
              {tk("ui.laptop.battery.rate_unit")}
              <span class="dim small">({b.rate_mw < 0 ? tk("ui.laptop.battery.discharging") : tk("ui.laptop.battery.charging")})</span>
            </dd>
          {/if}
          {#if b.design_mwh != null || b.full_charge_mwh != null}
            <dt>{tk("ui.laptop.battery.design")}</dt>
            <dd class="mono">{mwh(b.design_mwh)}</dd>
            <dt>{tk("ui.laptop.battery.full")}</dt>
            <dd class="mono">{mwh(b.full_charge_mwh)}</dd>
            {#if wearPct != null}
              <dt>{tk("ui.laptop.battery.wear")}</dt>
              <dd class="mono" class:warn={wearPct >= 30} class:ok={wearPct < 15}>{wearPct}%</dd>
            {/if}
          {/if}
          {#if b.cycle_count != null}
            <dt>{tk("ui.laptop.battery.cycles")}</dt>
            <dd class="mono">{b.cycle_count}</dd>
          {/if}
        </dl>
      </Card>
    {:else}
      <Card>
        <StateView kind="empty" title={tk("ui.laptop.not_laptop")} text={tk("ui.laptop.not_laptop_text")} />
      </Card>
    {/if}

    <!-- ── Знахідки батареї ── -->
    {#if view.findings.length}
      <div class="list">
        {#each view.findings as f (f.id)}<FindingCard finding={f} />{/each}
      </div>
    {/if}

    <!-- ── Режим живлення ── -->
    <Card title={tk("ui.laptop.power_mode.title")} subtitle={tk("ui.laptop.power_mode.subtitle")}>
      {#if view.overlay_supported}
        <p class="small dim note">{tk("ui.laptop.power_mode.note")}</p>
        <Segmented
          label={tk("ui.laptop.power_mode.title")}
          bind:value={overlayMode}
          options={[
            { value: "economy",  label: tk("ui.laptop.power_mode.economy") },
            { value: "balanced", label: tk("ui.laptop.power_mode.balanced") },
            { value: "better",   label: tk("ui.laptop.power_mode.better") },
            { value: "best",     label: tk("ui.laptop.power_mode.best") },
          ]}
        />
        <div class="row-action">
          <Button
            variant="primary"
            icon="check"
            loading={applyingOverlay}
            disabled={applyingOverlay}
            onclick={applyOverlay}
          >
            {tk("ui.laptop.power_mode.apply")}
          </Button>
        </div>
      {:else}
        <StateView kind="empty" title={tk("ui.laptop.power_mode.not_supported")} />
      {/if}
    </Card>

    <!-- ── GPU для гри ── -->
    <Card title={tk("ui.laptop.gpu.title")} subtitle={tk("ui.laptop.gpu.subtitle")}>
      <div class="gpu-form">
        <div class="field">
          <label class="small muted" for="gpu-exe">{tk("ui.laptop.gpu.exe_label")}</label>
          <input
            id="gpu-exe"
            class="mono"
            bind:value={gpuExe}
            placeholder={tk("ui.laptop.gpu.exe_placeholder")}
          />
        </div>
        <Segmented
          label={tk("ui.laptop.gpu.title")}
          bind:value={gpuPref}
          options={[
            { value: "default",          label: tk("ui.laptop.gpu.default") },
            { value: "power_saving",     label: tk("ui.laptop.gpu.power_saving") },
            { value: "high_performance", label: tk("ui.laptop.gpu.high_perf") },
          ]}
        />
        <div class="row-action">
          <Button
            variant="primary"
            icon="check"
            loading={applyingGpu}
            disabled={applyingGpu || !gpuExe.trim()}
            onclick={applyGpu}
          >
            {tk("ui.laptop.gpu.apply")}
          </Button>
        </div>
      </div>
    </Card>

    {#if error}<p class="small warn">{error}</p>{/if}
  {/if}

  {#snippet secondary()}
    <Button variant="ghost" icon="undo" onclick={load}>{tk("ui.laptop.refresh")}</Button>
  {/snippet}
</Page>

<style>
  .kv {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 1.5) calc(var(--space) * 5);
    margin: 0;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
  }
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .note {
    margin: 0 0 calc(var(--space) * 4);
  }
  .row-action {
    margin-top: calc(var(--space) * 5);
    display: flex;
    justify-content: flex-end;
  }
  .gpu-form {
    display: grid;
    gap: calc(var(--space) * 4);
  }
  .field {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  input {
    padding: calc(var(--space) * 2.5) calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text);
  }
  input:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 1px;
  }
  .warn {
    color: var(--warn);
  }
  .ok {
    color: var(--ok);
  }
</style>
