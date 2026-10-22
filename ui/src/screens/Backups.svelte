<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { JournalEntry } from "../lib/bindings/JournalEntry";
  import { dateTime } from "../lib/format";
  import { t, tk } from "../lib/i18n";
  import { flashBg, logLocal } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import Modal from "../components/Modal.svelte";
  import Page from "../components/Page.svelte";
  import StateView from "../components/StateView.svelte";

  let entries = $state<JournalEntry[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let confirm = $state<{ id: number | null } | null>(null);
  let busy = $state(false);

  async function load() {
    loading = true;
    try {
      entries = (await api.journal()).slice().reverse();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }
  onMount(load);

  const active = $derived(entries.filter((e) => e.state.state === "applied"));

  async function doRollback() {
    if (!confirm) return;
    busy = true;
    error = null;
    try {
      if (confirm.id === null) {
        const r = await api.rollbackAll();
        logLocal(r.failed.length ? "warn" : "info", tk("ui.backups.rolled_all", { n: r.rolled_back.length, f: r.failed.length }));
        for (const f of r.failed) logLocal("error", `#${f.id}: ${f.error}`);
      } else {
        await api.rollback(confirm.id);
        logLocal("info", tk("ui.backups.rolled_one", { id: confirm.id }));
      }
      flashBg("ok");
    } catch (e) {
      error = asCmdError(e).message;
      flashBg("warn");
    } finally {
      busy = false;
      confirm = null;
      await load();
    }
  }

  function location(e: JournalEntry): string {
    const tg = e.target;
    switch (tg.kind) {
      case "registry":
        return `${tg.hive === "LocalMachine" ? "HKLM" : "HKCU"}\\${tg.path}\\${tg.name}`;
      case "process_priority":
        return `${tg.process.name} (PID ${tg.process.pid})`;
      case "service":
        return tg.name;
      default:
        return tk(`ui.target.${tg.kind}`);
    }
  }

  function value(v: JournalEntry["old"]): string {
    switch (v.kind) {
      case "registry":
        return v.value ? `${v.value.type} ${JSON.stringify(v.value.value)}` : tk("tweak.value.absent");
      case "power_scheme":
        return v.value;
      case "priority":
        return tk(`priority.${v.value}`);
      case "service":
        return typeof v.value === "string" ? tk(`service.${v.value}`) : JSON.stringify(v.value);
      default:
        return "—";
    }
  }
</script>

<Page title={tk("ui.backups.title")} subtitle={tk("ui.backups.subtitle")}>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  {#if loading}
    <Card><StateView kind="loading" title={tk("ui.common.loading")} /></Card>
  {:else if !entries.length}
    <Card><StateView kind="empty" title={tk("ui.backups.empty_title")} text={tk("ui.backups.empty_text")} /></Card>
  {:else}
    <Card padded={false}>
      <table>
        <thead>
          <tr>
            <th>#</th><th>{tk("ui.backups.when")}</th><th>{tk("ui.backups.what")}</th><th>{tk("ui.backups.change")}</th><th>{tk("ui.backups.state")}</th><th></th>
          </tr>
        </thead>
        <tbody>
          {#each entries as e (e.id)}
            <tr>
              <td class="dim">{e.id}</td>
              <td class="small">{dateTime(e.created_unix)}</td>
              <td>
                <div>{t({ key: `tweak.${e.tweak_id}.title`, args: {} })}</div>
                <div class="mono small muted">{location(e)}</div>
              </td>
              <td class="mono small">{value(e.old)} → {value(e.new)}</td>
              <td><span class="state {e.state.state}">{tk(`ui.backups.state.${e.state.state}`)}</span>
                {#if e.state.state === "failed"}<div class="small err">{e.state.error}</div>{/if}
              </td>
              <td>
                {#if e.state.state === "applied"}
                  <Button small icon="undo" onclick={() => (confirm = { id: e.id })}>{tk("ui.backups.rollback")}</Button>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </Card>
  {/if}

  {#snippet secondary()}
    <span class="small muted">{tk("ui.backups.active", { n: active.length })}</span>
  {/snippet}
  {#snippet primary()}
    <Button variant="danger" icon="undo" disabled={!active.length} onclick={() => (confirm = { id: null })}>{tk("ui.backups.rollback_all")}</Button>
  {/snippet}
</Page>

{#if confirm}
  <Modal
    title={confirm.id === null ? tk("ui.backups.confirm_all", { n: active.length }) : tk("ui.backups.confirm_one", { id: confirm.id })}
    confirmLabel={tk("ui.backups.rollback")}
    danger
    {busy}
    onconfirm={doRollback}
    oncancel={() => (confirm = null)}
  >
    <p>{tk("ui.backups.confirm_text")}</p>
  </Modal>
{/if}

<style>
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th {
    text-align: left;
    color: var(--text-muted);
    font-weight: 500;
    padding: calc(var(--space) * 4) calc(var(--space) * 4) calc(var(--space) * 2);
  }
  td {
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-top: 1px solid var(--border);
    vertical-align: top;
  }
  .state {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    font-size: 12px;
    color: var(--text-muted);
  }
  .state.applied {
    color: var(--ok);
    border-color: var(--ok);
  }
  .state.failed {
    color: var(--danger);
    border-color: var(--danger);
  }
  .state.pending {
    color: var(--warn);
    border-color: var(--warn);
  }
  .err {
    color: var(--danger);
  }
</style>
