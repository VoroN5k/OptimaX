<script lang="ts">
  import { api } from "../lib/api";
  import type { StartMode } from "../lib/bindings/StartMode";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Segmented from "./Segmented.svelte";

  // Як починати запис: автоматично (гра в фокусі) або за гарячою клавішею. Зберігається в налаштуваннях.
  async function save(_v: StartMode) {
    await api.setSettings($state.snapshot(ui.settings));
  }
</script>

<div class="picker">
  <span class="small muted">{tk("ui.rec.start")}</span>
  <Segmented
    label={tk("ui.rec.start")}
    bind:value={ui.settings.record_start}
    onchange={save}
    options={[
      { value: "auto", label: tk("ui.rec.mode.auto") },
      { value: "hotkey", label: tk("ui.rec.mode.hotkey") },
    ]}
  />
  <p class="small dim">{tk(ui.settings.record_start === "hotkey" ? "ui.rec.mode.hotkey_hint" : "ui.rec.mode.auto_hint")}</p>
</div>

<style>
  .picker {
    display: grid;
    gap: calc(var(--space) * 2);
    max-width: 420px;
  }
</style>
