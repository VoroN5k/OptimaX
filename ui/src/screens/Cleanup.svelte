<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { CleanupCategory } from "../lib/bindings/CleanupCategory";
  import type { DriveView } from "../lib/bindings/DriveView";
  import type { DryRunView } from "../lib/bindings/DryRunView";
  import type { Item } from "../lib/bindings/Item";
  import { size } from "../lib/format";
  import { tk } from "../lib/i18n";
  import { cl, flashBg, logLocal } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import CleanupGrowth from "../components/CleanupGrowth.svelte";
  import CleanupJournal from "../components/CleanupJournal.svelte";
  import CleanupQuarantine from "../components/CleanupQuarantine.svelte";
  import CleanupRow from "../components/CleanupRow.svelte";
  import Icon from "../components/Icon.svelte";
  import Modal from "../components/Modal.svelte";
  import Page from "../components/Page.svelte";
  import Ring from "../components/Ring.svelte";
  import Segmented from "../components/Segmented.svelte";
  import StateView from "../components/StateView.svelte";

  type Tab = "now" | "growth" | "quarantine" | "journal";
  const CATEGORIES: CleanupCategory[] = [
    "temp",
    "windows_update",
    "drivers",
    "browser_caches",
    "dev_caches",
    "shader_caches",
    "crash_dumps",
    "system",
  ];

  let drives = $state<DriveView[]>([]);
  let drive = $state("");
  let full = $state(1);
  let tab = $state<Tab>("now");
  let selected = $state<string[]>([]);
  let dry = $state<DryRunView | null>(null);
  let confirm = $state<DryRunView | null>(null);
  let restoreAsk = $state<{ reason: string; retry: () => Promise<void> } | null>(null);
  let toolAsk = $state<"dism" | "hibernate" | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let journalKey = $state(0);
  /** Що зараз виконується (точка відновлення, DISM, powercfg) — щоб довгі кроки було видно. */
  let running = $state<string | null>(null);
  /** Група «Відоме, не чіпати» згорнута за замовчуванням: там нічого робити. */
  let showZones = $state(false);
  /** «Невідоме»: спершу найбільші, решта — за кнопкою. */
  const UNKNOWN_SHOWN = 10;
  let showAllUnknown = $state(false);

  /** Помилка: у банер (видно з будь-якого місця сторінки) і в «Журнал дій». */
  function fail(msg: string) {
    error = msg;
    logLocal("error", msg);
  }

  onMount(async () => {
    try {
      drives = await api.cleanupDrives();
      drive = drives.find((d) => d.is_system)?.root ?? drives[0]?.root ?? "";
      if (!cl.view) cl.view = await api.cleanupLastScan();
      if (cl.view) drive = cl.view.report.drive;
    } catch (e) {
      fail(asCmdError(e).message);
    }
  });

  const view = $derived(cl.view && cl.view.report.drive === drive ? cl.view : null);
  const report = $derived(view?.report ?? null);
  const current = $derived(view?.drive ?? drives.find((d) => d.root === drive) ?? null);
  const usedPct = $derived(current && current.total_bytes ? (1 - current.free_bytes / current.total_bytes) * 100 : 0);
  const byCategory = $derived.by(() => {
    const m = new Map<CleanupCategory, Item[]>();
    for (const c of CATEGORIES) m.set(c, []);
    for (const i of report?.items ?? []) m.get(i.category)?.push(i);
    return m;
  });
  const selectedItems = $derived((report?.items ?? []).filter((i) => i.selectable && selected.includes(i.path)));
  const selectedBytes = $derived(selectedItems.reduce((s, i) => s + i.eligible_bytes, 0));
  const locked = $derived(cl.scanning || cl.executing || busy);

  // Після очищення — нова картина диска: вибір і сухий прогін застаріли.
  $effect(() => {
    if (cl.lastExec) {
      untrack(() => {
        selected = [];
        dry = null;
        journalKey++;
        // Показати підсумок і одразу нову картину диска (той самий режим скану).
        document.querySelector(".page .scroll")?.scrollTo({ top: 0, behavior: "smooth" });
        void rescan();
      });
    }
  });

  async function startScan() {
    error = null;
    cl.error = null;
    cl.lastExec = null;
    selected = [];
    dry = null;
    try {
      await api.cleanupScanStart(drive, full === 1);
      cl.scanning = true;
      cl.progress = null;
      logLocal("info", tk("ui.clean.scan_log", { drive }));
    } catch (e) {
      fail(asCmdError(e).message);
    }
  }

  /** Повторний скан після очищення: підсумок (`cl.lastExec`) лишається видимим. */
  async function rescan() {
    const mode = cl.view?.report.full ?? full === 1;
    try {
      await api.cleanupScanStart(drive, mode);
      cl.scanning = true;
      cl.progress = null;
    } catch (e) {
      fail(asCmdError(e).message);
    }
  }

  function toggle(path: string, on: boolean) {
    selected = on ? [...selected, path] : selected.filter((p) => p !== path);
    dry = null;
  }

  async function runDry(openConfirm: boolean) {
    error = null;
    busy = true;
    try {
      const d = await api.cleanupDryRun(selected);
      dry = d;
      if (openConfirm) confirm = d;
      logLocal("info", tk("ui.clean.dry_log", { n: d.files, size: size(d.delete_bytes + d.quarantine_bytes) }));
    } catch (e) {
      fail(asCmdError(e).message);
    } finally {
      busy = false;
    }
  }

  async function execute(planId: number, allow: boolean, needsRestore: boolean) {
    error = null;
    cl.error = null;
    busy = true;
    running = needsRestore ? tk("ui.clean.running_restore") : null;
    // До виклику: подія «завершено» може прийти раніше, ніж відповідь команди.
    cl.lastExec = null;
    cl.execProgress = null;
    cl.executing = true;
    try {
      await api.cleanupExecute(planId, allow);
      confirm = null;
    } catch (e) {
      cl.executing = false;
      const err = asCmdError(e);
      if (err.kind === "restore_point") {
        confirm = null;
        restoreAsk = { reason: err.message, retry: () => execute(planId, true, false) };
      } else {
        fail(err.message);
      }
    } finally {
      busy = false;
      running = null;
    }
  }

  async function runTool(kind: "dism" | "hibernate", allow: boolean) {
    error = null;
    busy = true;
    toolAsk = null;
    running = tk(`ui.clean.running_${kind}`);
    try {
      const rec = kind === "dism" ? await api.cleanupDism(allow) : await api.cleanupHibernate(false, allow);
      logLocal("info", tk(`ui.clean.${kind}_done`, { size: size(rec.freed_bytes) }));
      flashBg("ok");
      journalKey++;
      await startScan();
    } catch (e) {
      const err = asCmdError(e);
      if (err.kind === "restore_point") {
        restoreAsk = { reason: err.message, retry: () => runTool(kind, true) };
      } else {
        fail(err.message);
        flashBg("warn");
      }
    } finally {
      busy = false;
      running = null;
    }
  }

  async function reveal(path: string) {
    try {
      await api.cleanupReveal(path);
    } catch (e) {
      fail(asCmdError(e).message);
    }
  }

  const zonesBytes = $derived((report?.protected_zones ?? []).reduce((s, i) => s + i.bytes, 0));
  const catBytes = (items: Item[]) => items.filter((i) => i.selectable).reduce((s, i) => s + i.eligible_bytes, 0);
  const share = $derived(
    cl.execProgress && cl.execProgress.total ? Math.min(1, cl.execProgress.done / cl.execProgress.total) : 0,
  );
</script>

{#snippet rows(items: Item[])}
  {#each items as item (item.path)}
    <CleanupRow
      {item}
      checked={selected.includes(item.path)}
      disabled={locked}
      ontoggle={(on) => toggle(item.path, on)}
      onreveal={() => reveal(item.path)}
      onrevealpath={reveal}
      ondism={() => (toolAsk = "dism")}
      onhibernate={() => (toolAsk = "hibernate")}
    />
  {/each}
{/snippet}

{#snippet dryList(d: DryRunView, withSamples: boolean)}
  <ul class="dry">
    {#each d.items as i (i.path)}
      <li>
        <div class="dry-top">
          <span class="mono dpath">{i.path}</span>
          <span class="tag {i.action}">{tk(`ui.clean.action.${i.action}`)}</span>
          <span class="mono">{tk("ui.clean.files_n", { n: i.files })} · {size(i.bytes)}</span>
        </div>
        {#if withSamples && i.sample.length}
          <details>
            <summary class="small muted">{tk("ui.clean.sample", { n: i.sample.length, total: i.files })}</summary>
            <div class="sample mono">
              {#each i.sample as f (f)}<div>{f}</div>{/each}
            </div>
          </details>
        {/if}
      </li>
    {/each}
  </ul>
  <div class="totals">
    {#if d.delete_bytes}<p>{tk("ui.clean.total_delete", { size: size(d.delete_bytes) })}</p>{/if}
    {#if d.quarantine_bytes}
      <p>{tk("ui.clean.total_quarantine", { size: size(d.quarantine_bytes), days: d.quarantine_days })}</p>
      <p class="small {d.quarantine_same_volume ? 'warn' : 'muted'}">
        {tk(d.quarantine_same_volume ? "ui.clean.q_same_volume" : "ui.clean.q_other_volume", { dir: d.quarantine_dir })}
      </p>
    {/if}
    {#if d.services.length}<p class="small muted">{tk("ui.clean.services", { list: d.services.join(", ") })}</p>{/if}
    {#if d.restore_point_pending}<p class="small muted">{tk("ui.clean.restore_point")}</p>{/if}
    {#if d.skipped_total}<p class="small muted">{tk("ui.clean.skipped_plan", { n: d.skipped_total })}</p>{/if}
    <p class="small dim">{tk("ui.clean.busy_note")}</p>
  </div>
{/snippet}

<Page title={tk("ui.clean.title")} subtitle={tk("ui.clean.subtitle")}>
  <Card>
    <div class="summary">
      <Ring value={usedPct} label={drive || "—"} caption={tk("ui.clean.used")} />
      <div class="sum-main">
        {#if view}
          <h2>{tk("ui.clean.found_safe", { size: size(view.safe_bytes) })}</h2>
          {#if view.consequences_bytes}
            <p class="warn-text">{tk("ui.clean.found_consequences", { size: size(view.consequences_bytes) })}</p>
          {/if}
          <p class="small muted">
            {tk("ui.clean.free", { free: size(current?.free_bytes), total: size(current?.total_bytes) })}
            {#if !cl.scanning}
              · {tk(report?.full ? "ui.clean.mode_full_done" : "ui.clean.mode_rules_done", { s: ((report?.duration_ms ?? 0) / 1000).toFixed(1) })}
            {/if}
          </p>
          {#if report?.cancelled}<p class="small warn-text">{tk("ui.clean.cancelled")}</p>{/if}
        {:else}
          <h2>{tk("ui.clean.not_scanned")}</h2>
          <p class="small muted">{tk("ui.clean.not_scanned_text")}</p>
        {/if}
        <div class="controls">
          {#if drives.length > 1}
            <Segmented
              label={tk("ui.clean.drive")}
              bind:value={drive}
              options={drives.map((d) => ({ value: d.root, label: d.root.replace("\\", "") }))}
            />
          {/if}
          <Segmented
            label={tk("ui.clean.mode")}
            bind:value={full}
            options={[
              { value: 0, label: tk("ui.clean.mode_rules") },
              { value: 1, label: tk("ui.clean.mode_full") },
            ]}
          />
          {#if cl.scanning}
            <Button variant="secondary" icon="stop" onclick={() => api.cleanupScanCancel()}>{tk("ui.clean.scan_stop")}</Button>
          {:else}
            <Button variant="secondary" icon="diagnosis" disabled={!drive || locked} onclick={startScan}>{tk("ui.clean.scan")}</Button>
          {/if}
        </div>
      </div>
    </div>
    {#if cl.scanning}
      <div class="progress" aria-live="polite">
        <span class="spin" aria-hidden="true"></span>
        <span class="small">
          {tk(cl.progress?.phase === "drive" ? "ui.clean.phase_drive" : "ui.clean.phase_rules")}
          · {tk("ui.clean.progress", { dirs: cl.progress?.dirs ?? 0, size: size(cl.progress?.bytes ?? 0) })}
        </span>
        <span class="small mono dim current" title={cl.progress?.current}>{cl.progress?.current ?? ""}</span>
      </div>
    {/if}
  </Card>

  {#if running}
    <Card>
      <div class="running" aria-live="polite"><span class="spin" aria-hidden="true"></span>{running}</div>
    </Card>
  {/if}

  {#if view?.rule_issues.length}
    <Card title={tk("ui.clean.rule_issues")}>
      <ul class="issues small mono">{#each view.rule_issues as i (i)}<li>{i}</li>{/each}</ul>
    </Card>
  {/if}

  {#if cl.executing}
    <Card title={tk("ui.clean.executing")}>
      <div class="track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(share * 100)}>
        <div class="fill" style:transform="scaleX({share})"></div>
      </div>
      <p class="small muted">
        {tk("ui.clean.exec_progress", {
          done: cl.execProgress?.done ?? 0,
          total: cl.execProgress?.total ?? 0,
          freed: size(cl.execProgress?.freed_bytes ?? 0),
          q: size(cl.execProgress?.quarantined_bytes ?? 0),
        })}
      </p>
    </Card>
  {:else if cl.lastExec}
    {@const r = cl.lastExec.record}
    <Card title={tk("ui.clean.done_title")}>
      <p>{tk("ui.clean.done_freed", { size: size(r.freed_bytes) })}</p>
      {#if r.quarantined_bytes}
        <p class="small {cl.lastExec.quarantine_same_volume ? 'warn-text' : 'muted'}">
          {tk(cl.lastExec.quarantine_same_volume ? "ui.clean.done_q_same" : "ui.clean.done_q_other", { size: size(r.quarantined_bytes) })}
        </p>
      {/if}
      {#if r.skipped}<p class="small muted">{tk("ui.clean.done_skipped", { n: r.skipped })}</p>{/if}
      {#if r.note}<p class="small muted">{r.note}</p>{/if}
      {#if cl.lastExec.cancelled}<p class="small warn-text">{tk("ui.clean.done_cancelled")}</p>{/if}
      <p class="small dim">{tk("ui.clean.done_rescan")}</p>
    </Card>
  {/if}

  <Segmented
    label={tk("ui.clean.tabs")}
    bind:value={tab}
    options={[
      { value: "now", label: tk("ui.clean.tab.now") },
      { value: "growth", label: tk("ui.clean.tab.growth") },
      { value: "quarantine", label: tk("ui.clean.tab.quarantine") },
      { value: "journal", label: tk("ui.clean.tab.journal") },
    ]}
  />

  {#if tab === "now"}
    {#if cl.scanning && !view}
      <Card><StateView kind="loading" title={tk("ui.clean.scanning")} /></Card>
    {:else if !view}
      <Card><StateView kind="empty" title={tk("ui.clean.empty_title")} text={tk("ui.clean.empty_text")} /></Card>
    {:else}
      {#if dry}
        <Card title={tk("ui.clean.dry_title")} subtitle={tk("ui.clean.dry_subtitle")}>
          {@render dryList(dry, true)}
        </Card>
      {/if}
      {#each CATEGORIES as cat (cat)}
        {@const items = byCategory.get(cat) ?? []}
        <Card title={tk(`ui.clean.cat.${cat}`)} subtitle={items.length ? tk("ui.clean.cat_sum", { size: size(catBytes(items)), n: items.length }) : tk("ui.clean.cat_empty")}>
          {#if items.length}<div class="rows">{@render rows(items)}</div>{/if}
        </Card>
      {/each}
      <Card title={tk("ui.clean.cat.unknown")} subtitle={tk("ui.clean.unknown_subtitle")}>
        {#if !report?.full}
          <p class="small muted">{tk("ui.clean.unknown_need_full")}</p>
        {:else if report.unknown.length}
          <div class="rows">{@render rows(showAllUnknown ? report.unknown : report.unknown.slice(0, UNKNOWN_SHOWN))}</div>
          {#if report.unknown.length > UNKNOWN_SHOWN}
            <div class="more">
              <Button small variant="ghost" onclick={() => (showAllUnknown = !showAllUnknown)}>
                {showAllUnknown ? tk("ui.clean.unknown_less") : tk("ui.clean.unknown_more", { n: report.unknown.length - UNKNOWN_SHOWN })}
              </Button>
            </div>
          {/if}
        {:else}
          <p class="small muted">{tk("ui.clean.unknown_none")}</p>
        {/if}
      </Card>
      {#if report?.full && report.protected_zones.length}
        <Card
          title={tk("ui.clean.zones_title")}
          subtitle={tk("ui.clean.zones_subtitle", { n: report.protected_zones.length, size: size(zonesBytes) })}
        >
          {#snippet actions()}
            <Button small variant="ghost" onclick={() => (showZones = !showZones)}>
              {tk(showZones ? "ui.clean.zones_hide" : "ui.clean.zones_show")}
            </Button>
          {/snippet}
          {#if showZones}<div class="rows">{@render rows(report.protected_zones)}</div>{/if}
        </Card>
      {/if}
      {#if view.suggest_move}
        <Card title={tk("ui.clean.move_title")}>
          <p class="muted">{tk("ui.clean.move_text")}</p>
        </Card>
      {/if}
    {/if}
  {:else if tab === "growth"}
    {#key drive + (cl.view?.report.started_unix ?? 0)}
      <CleanupGrowth {drive} onreveal={reveal} />
    {/key}
  {:else if tab === "quarantine"}
    {#key journalKey}<CleanupQuarantine onchange={() => journalKey++} />{/key}
  {:else}
    {#key journalKey}<CleanupJournal onchange={() => journalKey++} />{/key}
  {/if}

  {#snippet secondary()}
    {#if cl.executing}
      <Button variant="secondary" icon="stop" onclick={() => api.cleanupExecCancel()}>{tk("ui.clean.exec_stop")}</Button>
    {:else}
      <Button variant="secondary" icon="diagnosis" disabled={!selectedItems.length || locked} onclick={() => runDry(false)}>
        {tk("ui.clean.dry_run")}
      </Button>
    {/if}
  {/snippet}
  {#snippet primary()}
    <Button variant="primary" icon="trash" disabled={!selectedItems.length || locked} loading={busy && !confirm} onclick={() => runDry(true)}>
      {tk("ui.clean.clean_selected", { size: size(selectedBytes) })}
    </Button>
  {/snippet}
</Page>

{#if confirm}
  {@const c = confirm}
  <Modal
    title={tk("ui.clean.confirm_title")}
    confirmLabel={tk("ui.clean.confirm", { size: size(c.delete_bytes + c.quarantine_bytes) })}
    danger
    wide
    {busy}
    onconfirm={() => execute(c.plan_id, false, c.restore_point_pending)}
    oncancel={() => (confirm = null)}
  >
    <p>{tk("ui.clean.confirm_text")}</p>
    {@render dryList(c, false)}
    {#if running}<p class="running small" aria-live="polite"><span class="spin" aria-hidden="true"></span>{running}</p>{/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </Modal>
{/if}

{#if error || cl.error}
  <div class="toast" role="alert">
    <Icon name="warning" size={18} />
    <span>{error ?? cl.error}</span>
    <button
      type="button"
      class="close"
      aria-label={tk("ui.clean.dismiss")}
      onclick={() => {
        error = null;
        cl.error = null;
      }}><Icon name="close" size={16} /></button
    >
  </div>
{/if}

{#if restoreAsk}
  {@const r = restoreAsk}
  <Modal
    title={tk("ui.clean.rp_title")}
    confirmLabel={tk("ui.clean.rp_continue")}
    danger
    {busy}
    onconfirm={() => {
      // Спершу беремо функцію: після `restoreAsk = null` значення `r` вже порожнє.
      const retry = r.retry;
      restoreAsk = null;
      void retry();
    }}
    oncancel={() => (restoreAsk = null)}
  >
    <p>{tk("ui.clean.rp_text")}</p>
    <p class="mono small">{r.reason}</p>
  </Modal>
{/if}

{#if toolAsk}
  {@const k = toolAsk}
  <Modal
    title={tk(`ui.clean.${k}_title`)}
    confirmLabel={tk(`ui.clean.${k}_confirm`)}
    danger
    {busy}
    onconfirm={() => runTool(k, false)}
    oncancel={() => (toolAsk = null)}
  >
    <p>{tk(`ui.clean.${k}_text`)}</p>
    <p class="small">{tk(`ui.clean.${k}_command`)}</p>
    <p class="small muted">{tk("ui.clean.restore_point")}</p>
  </Modal>
{/if}

<style>
  .summary {
    display: flex;
    gap: calc(var(--space) * 8);
    align-items: center;
  }
  .sum-main {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .sum-main h2 {
    font-size: 20px;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 3);
    align-items: center;
    margin-top: calc(var(--space) * 2);
  }
  .progress {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    margin-top: calc(var(--space) * 4);
    padding-top: calc(var(--space) * 3);
    border-top: 1px solid var(--border);
    min-width: 0;
  }
  .current {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spin {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    border-radius: 50%;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent-from);
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .rows {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .more {
    display: flex;
    justify-content: center;
    padding-top: calc(var(--space) * 2);
  }
  .warn-text,
  .warn {
    color: var(--warn);
  }
  .err {
    color: var(--danger);
    margin: 0;
  }
  .issues {
    margin: 0;
    padding-left: calc(var(--space) * 5);
    color: var(--warn);
  }
  .track {
    height: 6px;
    margin-bottom: calc(var(--space) * 3);
    border-radius: 999px;
    background: var(--bg-input);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    transform-origin: left;
    background: var(--accent-gradient);
    transition: transform var(--dur-slow) var(--ease);
  }
  .dry {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: calc(var(--space) * 2);
  }
  .dry li {
    min-width: 0;
    padding: calc(var(--space) * 2) calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--bg-input);
  }
  .dry-top {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 1) calc(var(--space) * 3);
    align-items: center;
    min-width: 0;
  }
  /* Шлях повністю, з переносом — без горизонтальної прокрутки. */
  .dpath {
    flex: 1 1 100%;
    min-width: 0;
    overflow-wrap: anywhere;
    color: var(--text);
  }
  .running {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    margin: 0;
    color: var(--text);
  }
  .toast {
    position: fixed;
    z-index: 60;
    right: calc(var(--space) * 8);
    bottom: calc(var(--space) * 22);
    max-width: min(560px, calc(100vw - 280px));
    display: flex;
    align-items: flex-start;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-radius: var(--radius-md);
    border: 1px solid var(--danger);
    background: color-mix(in srgb, var(--danger) 14%, var(--bg-card));
    box-shadow: var(--shadow-modal);
    color: var(--text);
    animation: page-in var(--dur) var(--ease);
  }
  .toast :global(svg) {
    flex-shrink: 0;
    color: var(--danger);
    margin-top: 2px;
  }
  .toast span {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .close {
    border: none;
    background: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
  }
  .close:hover {
    color: var(--text);
  }
  .tag {
    flex-shrink: 0;
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid currentColor;
  }
  .tag.delete {
    color: var(--ok);
  }
  .tag.quarantine {
    color: var(--warn);
  }
  details {
    margin-top: calc(var(--space) * 2);
  }
  summary {
    cursor: pointer;
  }
  .sample {
    max-height: 200px;
    overflow: auto;
    margin-top: calc(var(--space) * 1);
    font-size: 11.5px;
    color: var(--text-muted);
  }
  .totals {
    display: grid;
    gap: calc(var(--space) * 1);
    margin-top: calc(var(--space) * 3);
  }
  .totals p {
    margin: 0;
  }
  @media (prefers-reduced-motion: reduce) {
    .fill {
      transition: none;
    }
  }
</style>
