<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { ServiceView } from "../lib/bindings/ServiceView";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";

  // Підказка на «Головній»: програма встановлена в Program Files, а служби немає — тож Windows
  // питає UAC при кожному запуску. Встановлення — лише через згоду в картці служби
  // («Налаштування»); тут тільки перехід туди. «Не показувати» — назавжди (у налаштуваннях).
  let view = $state<ServiceView | null>(null);

  onMount(async () => {
    if (ui.settings.service_hint_dismissed) return;
    try {
      view = await api.serviceStatus();
    } catch {
      view = null;
    }
  });

  // Портативна копія (не з Program Files) служби не отримає — їй підказка нічого не дасть.
  const show = $derived(
    view !== null &&
      view.state === "not_installed" &&
      view.from_service_exe &&
      !ui.settings.service_hint_dismissed &&
      !ui.info?.service,
  );

  function install() {
    ui.serviceInstall = true;
    ui.screen = "settings";
  }

  function dismiss() {
    ui.settings.service_hint_dismissed = true;
    void api.setSettings($state.snapshot(ui.settings)).catch(() => undefined);
  }
</script>

{#if show}
  <div class="hint" role="status">
    <i class="dot" aria-hidden="true"></i>
    <div class="text">
      <strong>{tk("ui.service.hint.title")}</strong>
      <span class="small muted">{tk("ui.service.hint.text")}</span>
    </div>
    <div class="actions">
      <Button small variant="secondary" onclick={dismiss}>{tk("ui.service.hint.dismiss")}</Button>
      <Button small variant="primary" onclick={install}>{tk("ui.service.hint.install")}</Button>
    </div>
  </div>
{/if}

<style>
  .hint {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: calc(var(--space) * 3);
    margin-bottom: calc(var(--space) * 4);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--info);
    border-radius: var(--radius-md);
    background: var(--bg-card);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--info);
    flex-shrink: 0;
  }
  .text {
    flex: 1;
    min-width: 240px;
    display: grid;
    gap: 2px;
  }
  .actions {
    display: flex;
    gap: calc(var(--space) * 2);
  }
</style>
