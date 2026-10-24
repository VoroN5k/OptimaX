<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { CleanupSettings } from "../lib/bindings/CleanupSettings";
  import type { QuarantineView } from "../lib/bindings/QuarantineView";
  import { dateTime, size } from "../lib/format";
  import { tk } from "../lib/i18n";
  import { logLocal, ui } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import Card from "./Card.svelte";
  import Modal from "./Modal.svelte";
  import Segmented from "./Segmented.svelte";
  import StateView from "./StateView.svelte";
  import Toggle from "./Toggle.svelte";

  // Карантин для 🟡. Чесно про місце: на тому ж диску воно не звільняється до кінця терміну.
  let { onchange }: { onchange: () => void } = $props();

  let view = $state<QuarantineView | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let purgeAsk = $state<string | null>(null);

  async function load() {
    try {
      view = await api.cleanupQuarantine();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }
  onMount(load);

  async function save(patch: Partial<CleanupSettings>) {
    if (!view) return;
    error = null;
    const cleanup = { ...view.settings, ...patch };
    try {
      const settings = { ...ui.settings, cleanup };
      await api.setSettings(settings);
      ui.settings = settings;
      await load();
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  async function act(id: string, kind: "restore" | "purge") {
    busy = true;
    error = null;
    purgeAsk = null;
    try {
      const r =
        kind === "restore" ? await api.cleanupQuarantineRestore(id) : await api.cleanupQuarantinePurge(id);
      logLocal(
        r.skipped ? "warn" : "info",
        tk(`ui.clean.q_${kind}_log`, { size: size(kind === "restore" ? r.restored_bytes : r.freed_bytes), n: r.skipped }),
      );
      onchange();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      busy = false;
    }
  }

  const driveOptions = $derived([
    { value: "", label: tk("ui.clean.q_drive_system") },
    ...(view?.other_drives ?? []).map((d) => ({ value: d.root, label: `${d.root.replace("\\", "")} · ${size(d.free_bytes)}` })),
  ]);
</script>

{#if loading}
  <Card><StateView kind="loading" title={tk("ui.common.loading")} /></Card>
{:else if !view}
  <Card><StateView kind="error" title={tk("ui.common.error")} text={error ?? ""} /></Card>
{:else}
  {@const v = view}
  <Card title={tk("ui.clean.q_title")} subtitle={tk("ui.clean.q_subtitle")}>
    {#if v.on_system_drive}
      <p class="warn small">{tk("ui.clean.q_system_warn")}</p>
    {/if}
    <dl class="kv">
      <dt>{tk("ui.clean.q_dir")}</dt>
      <dd class="mono">{v.dir}</dd>
      <dt>{tk("ui.clean.q_drive")}</dt>
      <dd>
        <Segmented
          label={tk("ui.clean.q_drive")}
          value={v.settings.quarantine_drive ?? ""}
          options={driveOptions}
          onchange={(d) => save({ quarantine_drive: d || null })}
        />
      </dd>
      <dt>{tk("ui.clean.q_days")}</dt>
      <dd>
        <Segmented
          label={tk("ui.clean.q_days")}
          value={v.settings.quarantine_days}
          options={[3, 7, 14, 30].map((n) => ({ value: n, label: tk("ui.clean.days", { n }) }))}
          onchange={(n) => save({ quarantine_days: n })}
        />
      </dd>
      <dt>{tk("ui.clean.q_auto")}</dt>
      <dd class="auto">
        <Toggle label={tk("ui.clean.q_auto")} checked={v.settings.auto_purge} onchange={(on) => save({ auto_purge: on })} />
        <span class="small muted">{tk("ui.clean.q_auto_hint")}</span>
      </dd>
    </dl>
    {#if error}<p class="err small" role="alert">{error}</p>{/if}
  </Card>

  <Card title={tk("ui.clean.q_batches")}>
    {#if v.batches.length}
      <div class="list">
        {#each v.batches.slice().reverse() as b (b.id)}
          <div class="row">
            <div class="info">
              <span>{dateTime(b.created_unix)} · {tk("ui.clean.files_n", { n: b.files })} · <span class="mono">{size(b.bytes)}</span></span>
              <span class="small {b.expired ? 'warn' : 'muted'}">
                {tk(b.expired ? "ui.clean.q_expired" : "ui.clean.q_until", { date: dateTime(b.expires_unix) })}
                · {tk(b.same_volume ? "ui.clean.q_not_freed" : "ui.clean.q_freed")}
              </span>
            </div>
            <Button small variant="secondary" icon="undo" disabled={busy} onclick={() => act(b.id, "restore")}>{tk("ui.clean.q_restore")}</Button>
            <Button small variant="danger" icon="trash" disabled={busy} onclick={() => (purgeAsk = b.id)}>{tk("ui.clean.q_purge")}</Button>
          </div>
        {/each}
      </div>
    {:else}
      <StateView kind="empty" title={tk("ui.clean.q_empty")} />
    {/if}
  </Card>
{/if}

{#if purgeAsk}
  {@const id = purgeAsk}
  <Modal
    title={tk("ui.clean.q_purge_title")}
    confirmLabel={tk("ui.clean.q_purge")}
    danger
    {busy}
    onconfirm={() => act(id, "purge")}
    oncancel={() => (purgeAsk = null)}
  >
    <p>{tk("ui.clean.q_purge_text")}</p>
  </Modal>
{/if}

<style>
  .kv {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 3) calc(var(--space) * 5);
    align-items: center;
    margin: 0;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .auto {
    display: flex;
    gap: calc(var(--space) * 3);
    align-items: center;
  }
  .list {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 2.5) calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--bg-input);
  }
  .info {
    flex: 1;
    display: grid;
  }
  .warn {
    color: var(--warn);
  }
  .err {
    color: var(--danger);
  }
  p {
    margin: 0 0 calc(var(--space) * 3);
  }
</style>
