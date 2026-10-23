<script lang="ts">
  import type { Filter } from "../lib/bindings/Filter";
  import type { FitMode } from "../lib/bindings/FitMode";
  import type { UpscaleSettings } from "../lib/bindings/UpscaleSettings";
  import { tk } from "../lib/i18n";
  import Segmented from "./Segmented.svelte";
  import Slider from "./Slider.svelte";
  import Toggle from "./Toggle.svelte";

  // Налаштування апскейлу: фільтр, різкість (лише FSR / NIS), вписування, курсор.
  // Спільне для екрана «Апскейл» і профілю гри.
  let { settings = $bindable(), onchange }: { settings: UpscaleSettings; onchange?: () => void } = $props();

  const filters: Filter[] = ["nearest", "bilinear", "lanczos", "fsr", "nis"];
  const fits: FitMode[] = ["aspect", "integer", "stretch"];
  const sharp = $derived(settings.filter === "fsr" || settings.filter === "nis");
</script>

<div class="opts">
  <div class="row">
    <div class="label">
      <strong>{tk("ui.up.filter")}</strong>
      <p class="small muted">{tk(`ui.up.filter_hint.${settings.filter}`)}</p>
    </div>
    <Segmented
      label={tk("ui.up.filter")}
      bind:value={settings.filter}
      onchange={() => onchange?.()}
      options={filters.map((f) => ({ value: f, label: tk(`upscale.filter.${f}`) }))}
    />
  </div>

  {#if sharp}
    <div class="row">
      <div class="label">
        <strong>{tk("ui.up.sharpness")}</strong>
        <p class="small muted">{tk("ui.up.sharpness_hint")}</p>
      </div>
      <div class="slider">
        <Slider
          label={tk("ui.up.sharpness")}
          bind:value={settings.sharpness}
          format={(v) => `${Math.round(v * 100)}%`}
          onchange={() => onchange?.()}
        />
      </div>
    </div>
  {/if}

  <div class="row">
    <div class="label">
      <strong>{tk("ui.up.fit")}</strong>
      <p class="small muted">{tk("ui.up.fit_hint")}</p>
    </div>
    <Segmented
      label={tk("ui.up.fit")}
      bind:value={settings.fit}
      onchange={() => onchange?.()}
      options={fits.map((f) => ({ value: f, label: tk(`ui.up.fit.${f}`) }))}
    />
  </div>

  <div class="row">
    <div class="label">
      <strong>{tk("ui.up.cursor")}</strong>
      <p class="small muted">{tk("ui.up.cursor_hint")}</p>
    </div>
    <Toggle label={tk("ui.up.cursor")} bind:checked={settings.capture_cursor} onchange={() => onchange?.()} />
  </div>
</div>

<style>
  .opts {
    display: grid;
    gap: calc(var(--space) * 5);
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: calc(var(--space) * 6);
  }
  .label {
    max-width: 420px;
  }
  .label p {
    margin: calc(var(--space) * 1) 0 0;
  }
  .slider {
    width: 260px;
  }
</style>
