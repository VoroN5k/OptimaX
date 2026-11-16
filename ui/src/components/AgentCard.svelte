<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { AgentView } from "../lib/bindings/AgentView";
  import { tk } from "../lib/i18n";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import Modal from "./Modal.svelte";
  import StateView from "./StateView.svelte";
  import Toggle from "./Toggle.svelte";

  // Агент у треї (етап 4а): стан, автозапуск при вході в Windows лише після згоди, запуск зараз.
  // Що дозволено (встановлена копія, без прав адміністратора) — вирішує бекенд.
  let view = $state<AgentView | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let consent = $state(false);
  /** Стан перемикача (повертається назад, якщо згоди не дали). */
  let autostart = $state(false);

  async function run(f: () => Promise<AgentView>) {
    busy = true;
    error = null;
    try {
      view = await f();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      busy = false;
      autostart = view?.autostart ?? false;
    }
  }

  onMount(() => void run(api.agentStatus));

  function toggle(on: boolean) {
    if (on) {
      // Спершу згода; перемикач не вмикається, доки її немає.
      autostart = false;
      consent = true;
    } else {
      void run(() => api.agentSetAutostart(false));
    }
  }

  async function enable() {
    consent = false;
    await run(() => api.agentSetAutostart(true));
  }
</script>

<Card title={tk("ui.agent.title")} subtitle={tk("ui.agent.subtitle")}>
  <div class="rows">
    {#if view === null && busy}
      <StateView kind="loading" title={tk("ui.service.loading")} />
    {:else if view}
      <p class="small" aria-live="polite">
        <strong>{tk(view.running ? "ui.agent.state.running" : "ui.agent.state.stopped")}</strong>
      </p>
      {#if !view.available}
        <p class="small muted">{tk("ui.agent.unavailable")}</p>
      {:else}
        <div class="line">
          <span>{tk("ui.agent.autostart")}</span>
          <Toggle label={tk("ui.agent.autostart")} bind:checked={autostart} disabled={busy} onchange={toggle} />
        </div>
        {#if view.autostart}<p class="small dim">{tk("ui.agent.autostart_on")}</p>{/if}
        {#if !view.running && !view.can_start}
          <p class="small muted">{tk("ui.agent.elevated")}</p>
        {/if}
        {#if view.can_start}
          <div class="actions">
            <Button variant="secondary" icon="play" loading={busy} disabled={busy} onclick={() => run(api.agentStart)}>{tk("ui.agent.start")}</Button>
          </div>
        {/if}
      {/if}
    {/if}
    {#if error}<StateView kind="error" title={tk("ui.common.error")} text={error} />{/if}
  </div>
</Card>

{#if consent}
  <Modal title={tk("ui.agent.consent.title")} confirmLabel={tk("ui.agent.consent.ok")} onconfirm={enable} oncancel={() => (consent = false)}>
    <p>{tk("ui.agent.consent.what")}</p>
    <p>{tk("ui.agent.consent.where")}</p>
    <p>{tk("ui.agent.consent.undo")}</p>
  </Modal>
{/if}

<style>
  .rows {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
  }
  p {
    margin: 0;
  }
</style>
