<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { ServiceView } from "../lib/bindings/ServiceView";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import Modal from "./Modal.svelte";
  import StateView from "./StateView.svelte";

  // Служба OptimaX: стан, встановлення лише за згодою, перевірка зв'язку, видалення. Що вона вміє
  // й чи довіряє цьому exe — вирішує бекенд. Без прав і без служби — перезапуск з правами.
  let view = $state<ServiceView | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let consent = $state(false);
  let removing = $state(false);

  async function run(f: () => Promise<ServiceView>) {
    busy = true;
    error = null;
    try {
      view = await f();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      busy = false;
    }
  }

  let cardEl = $state<HTMLElement | null>(null);

  onMount(async () => {
    await run(api.serviceStatus);
    // Прийшли з підказки на «Головній» — одразу показуємо згоду (встановлення лише після неї).
    if (ui.serviceInstall) {
      ui.serviceInstall = false;
      cardEl?.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
      if (view?.state === "not_installed") consent = true;
    }
  });

  async function install() {
    consent = false;
    await run(api.serviceInstall);
  }

  async function uninstall() {
    removing = false;
    await run(api.serviceUninstall);
  }

  async function relaunch() {
    busy = true;
    error = null;
    try {
      await api.relaunchElevated();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      busy = false;
    }
  }

  // Програма без прав, а служба не відповідає цій копії — зміни системи не працюватимуть.
  const needsRights = $derived(ui.info !== null && !ui.info?.elevated && view !== null && view.ping === null);
</script>

<Card title={tk("ui.service.title")} subtitle={tk("ui.service.subtitle")}>
  <div class="rows" bind:this={cardEl}>
    {#if view === null && busy}
      <StateView kind="loading" title={tk("ui.service.loading")} />
    {:else if view}
      <p class="small" aria-live="polite">
        <strong>{tk(`ui.service.state.${view.state}`)}</strong>
        {#if view.ping}
          · {tk("ui.service.ping", { v: view.ping.version, s: view.ping.uptime_s })}
        {/if}
      </p>
      {#if view.state === "running" && !view.from_service_exe}
        <p class="small muted">{tk("ui.service.other_exe", { exe: view.exe ?? "—" })}</p>
      {/if}
      {#if view.ping && !view.same_version}
        <p class="small warn">{tk("ui.service.version_mismatch", { v: view.ping.version })}</p>
      {/if}
      {#if view.error}<p class="small warn">{tk("ui.service.no_link", { error: view.error })}</p>{/if}
      <p class="small dim">{tk("ui.service.stage1")}</p>
      {#if needsRights}
        <p class="small warn">{tk("ui.service.needs_rights")}</p>
      {/if}
      <div class="actions">
        <Button variant="secondary" icon="refresh" loading={busy} onclick={() => run(api.serviceStatus)}>{tk("ui.service.check")}</Button>
        {#if needsRights}
          <Button variant="primary" disabled={busy} onclick={relaunch}>{tk("ui.service.relaunch")}</Button>
        {/if}
        {#if view.state === "not_installed"}
          <Button variant="primary" icon="play" disabled={busy} onclick={() => (consent = true)}>{tk("ui.service.install")}</Button>
        {:else}
          {#if view.ping && !view.same_version}
            <Button variant="secondary" disabled={busy} onclick={() => (consent = true)}>{tk("ui.service.reinstall")}</Button>
          {/if}
          <Button variant="danger" icon="trash" disabled={busy} onclick={() => (removing = true)}>{tk("ui.service.uninstall")}</Button>
        {/if}
      </div>
    {/if}
    {#if error}<StateView kind="error" title={tk("ui.common.error")} text={error} />{/if}
  </div>
</Card>

{#if consent}
  <Modal title={tk("ui.service.consent.title")} confirmLabel={tk("ui.service.consent.ok")} onconfirm={install} oncancel={() => (consent = false)}>
    <p>{tk("ui.service.consent.what")}</p>
    <p>{tk("ui.service.consent.where", { exe: view?.exe ?? "—" })}</p>
    <p>{tk("ui.service.consent.risk")}</p>
    <p>{tk("ui.service.consent.undo")}</p>
  </Modal>
{/if}

{#if removing}
  <Modal title={tk("ui.service.remove.title")} confirmLabel={tk("ui.service.uninstall")} danger onconfirm={uninstall} oncancel={() => (removing = false)}>
    <p>{tk("ui.service.remove.text")}</p>
  </Modal>
{/if}

<style>
  .rows {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: calc(var(--space) * 2);
  }
  p {
    margin: 0;
  }
  .warn {
    color: var(--warn);
  }
</style>
