<script lang="ts">
  import { api, asCmdError } from "../lib/api";
  import type { AnimationLevel } from "../lib/bindings/AnimationLevel";
  import type { DotsStats } from "../lib/dots";
  import { tk } from "../lib/i18n";
  import { logLocal, ui } from "../lib/state.svelte";
  import { on } from "../lib/api";
  import { onMount } from "svelte";
  import Modal from "../components/Modal.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import Page from "../components/Page.svelte";
  import ReleaseNotes from "../components/ReleaseNotes.svelte";
  import Segmented from "../components/Segmented.svelte";
  import Toggle from "../components/Toggle.svelte";

  let { dotsStats }: { dotsStats: DotsStats | null } = $props();

  async function save() {
    await api.setSettings($state.snapshot(ui.settings));
  }

  // Звіт для відгуку: збирає бекенд, файл користувач переглядає й надсилає сам.
  let reportBusy = $state(false);
  let reportPath = $state<string | null>(null);
  let reportError = $state<string | null>(null);

  async function makeReport() {
    reportBusy = true;
    reportError = null;
    try {
      reportPath = await api.feedbackReport();
    } catch (e) {
      reportError = asCmdError(e).message;
    } finally {
      reportBusy = false;
    }
  }

  // Оновлення: перевіряє й встановлює бекенд (підпис інсталятора перевіряє плагін Tauri).
  let checking = $state(false);
  let updateError = $state<string | null>(null);
  let confirmUpdate = $state(false);
  let installing = $state(false);
  let progress = $state<{ downloaded: number; total: number | null } | null>(null);
  // Початок завантаження — для швидкості в діалозі.
  let downloadStart = 0;

  onMount(() => {
    const unlisten = on("update-progress", (p) => {
      if (!progress) downloadStart = performance.now();
      progress = { downloaded: Number(p.downloaded), total: p.total == null ? null : Number(p.total) };
    });
    return () => void unlisten.then((u) => u());
  });

  async function checkUpdate() {
    checking = true;
    updateError = null;
    try {
      ui.update = await api.updateCheck();
    } catch (e) {
      updateError = asCmdError(e).message;
    } finally {
      checking = false;
    }
  }

  async function installUpdate() {
    const v = ui.update?.available?.version;
    if (!v) return;
    installing = true;
    updateError = null;
    progress = null;
    try {
      // Після успішного запуску інсталятора Windows закриє програму — далі код може не дійти.
      await api.updateInstall(v);
    } catch (e) {
      updateError = asCmdError(e).message;
      logLocal("error", tk("ui.update.install_failed", { message: updateError }));
      installing = false;
      confirmUpdate = false;
    }
  }

  const mb = (b: number) => (b / (1024 * 1024)).toFixed(1);

  // Частка завантаженого (null — розмір невідомий) і швидкість, МБ/с.
  const share = $derived(
    progress?.total ? Math.min(1, progress.downloaded / progress.total) : null,
  );
  const speed = $derived.by(() => {
    if (!progress) return null;
    const s = (performance.now() - downloadStart) / 1000;
    return s > 0.5 ? progress.downloaded / (1024 * 1024) / s : null;
  });

  async function revealReport() {
    if (!reportPath) return;
    try {
      await api.feedbackReveal(reportPath);
    } catch (e) {
      reportError = asCmdError(e).message;
    }
  }
</script>

<Page title={tk("ui.settings.title")} subtitle={tk("ui.settings.subtitle")}>
  <Card title={tk("ui.settings.appearance")}>
    <div class="rows">
      <div class="row">
        <div>
          <strong>{tk("ui.settings.animation")}</strong>
          <p class="small muted">{tk("ui.settings.animation_hint")}</p>
        </div>
        <Segmented
          label={tk("ui.settings.animation")}
          bind:value={ui.settings.animation}
          onchange={(_v: AnimationLevel) => save()}
          options={[
            { value: "high", label: tk("ui.settings.anim.high") },
            { value: "low", label: tk("ui.settings.anim.low") },
            { value: "off", label: tk("ui.settings.anim.off") },
          ]}
        />
      </div>
      <div class="row">
        <div>
          <strong>{tk("ui.settings.economy")}</strong>
          <p class="small muted">{tk("ui.settings.economy_hint")}</p>
        </div>
        <Toggle label={tk("ui.settings.economy")} bind:checked={ui.settings.economy} onchange={() => save()} />
      </div>
      <div class="row">
        <div>
          <strong>{tk("ui.settings.record_sound")}</strong>
          <p class="small muted">{tk("ui.settings.record_sound_hint")}</p>
        </div>
        <Toggle label={tk("ui.settings.record_sound")} bind:checked={ui.settings.record_sound} onchange={() => save()} />
      </div>
      <div class="row">
        <div>
          <strong>{tk("ui.settings.overlay")}</strong>
          <p class="small muted">{tk("ui.settings.overlay_hint")}</p>
        </div>
        <Toggle label={tk("ui.settings.overlay")} bind:checked={ui.settings.overlay} onchange={() => save()} />
      </div>
      {#if dotsStats}
        <p class="small dim mono">
          {tk("ui.settings.anim_stats", { dots: dotsStats.dots, ms: dotsStats.avgFrameMs.toFixed(2), fps: dotsStats.fps.toFixed(0) })}
        </p>
      {/if}
    </div>
  </Card>

  <Card title={tk("ui.update.title")}>
    <div class="rows">
      <div class="row">
        <div>
          <strong>{tk("ui.update.auto")}</strong>
          <p class="small muted">{tk("ui.update.auto_hint")}</p>
        </div>
        <Toggle label={tk("ui.update.auto")} bind:checked={ui.settings.update_check_on_start} onchange={() => save()} />
      </div>
      <div class="actions">
        <Button variant="secondary" icon="refresh" loading={checking} disabled={installing} onclick={checkUpdate}>{tk("ui.update.check")}</Button>
        {#if ui.update?.available}
          {#if ui.update.installed}
            <Button variant="primary" icon="play" loading={installing} onclick={() => (confirmUpdate = true)}>
              {tk("ui.update.install", { v: ui.update.available.version })}
            </Button>
          {:else}
            <Button variant="primary" icon="folder" onclick={() => api.updateOpenPage()}>{tk("ui.update.open_page")}</Button>
          {/if}
        {/if}
      </div>
      <div aria-live="polite" class="small">
        {#if ui.update && !ui.update.available}
          <p>{tk("ui.update.latest", { v: ui.update.current })}</p>
        {:else if ui.update?.available}
          <p>{tk("ui.update.available", { v: ui.update.available.version, current: ui.update.current })}</p>
          {#if !ui.update.installed}<p class="muted">{tk("ui.update.portable")}</p>{/if}
          {#if ui.update.available.sections.length}
            <ReleaseNotes sections={ui.update.available.sections} />
            <button type="button" class="link small" onclick={() => api.updateOpenPage()}>{tk("ui.notes.github")}</button>
          {/if}
        {/if}
        {#if progress}
          <p>{progress.total ? tk("ui.update.progress", { done: mb(progress.downloaded), total: mb(progress.total) }) : tk("ui.update.progress_unknown", { done: mb(progress.downloaded) })}</p>
        {/if}
        {#if updateError}<p class="error">{tk("ui.update.error", { message: updateError })}</p>{/if}
      </div>
    </div>
  </Card>

  <Card title={tk("ui.feedback.title")}>
    <div class="rows">
      <p class="muted">{tk("ui.feedback.intro")}</p>
      <ul class="small muted list">
        <li>{tk("ui.feedback.item.system")}</li>
        <li>{tk("ui.feedback.item.diagnosis")}</li>
        <li>{tk("ui.feedback.item.journals")}</li>
        <li>{tk("ui.feedback.item.log")}</li>
      </ul>
      <p class="small muted">{tk("ui.feedback.privacy")}</p>
      <div class="actions">
        <Button variant="primary" icon="copy" loading={reportBusy} onclick={makeReport}>{tk("ui.feedback.create")}</Button>
        {#if reportPath}
          <Button variant="secondary" icon="folder" onclick={revealReport}>{tk("ui.clean.reveal")}</Button>
        {/if}
      </div>
      <div aria-live="polite">
        {#if reportPath}
          <p class="small">{tk("ui.feedback.saved")} <span class="mono path" title={reportPath}>{reportPath}</span></p>
        {/if}
        {#if reportError}
          <p class="small error">{tk("ui.feedback.error", { message: reportError })}</p>
        {/if}
      </div>
    </div>
  </Card>

  <Card title={tk("ui.settings.about")}>
    <dl class="kv">
      <dt>{tk("ui.settings.version")}</dt><dd>{ui.info?.version ?? "—"}</dd>
      <dt>{tk("ui.settings.rights")}</dt><dd>{ui.info?.elevated ? tk("ui.status.admin") : tk("ui.status.no_admin")}</dd>
      <dt>PresentMon</dt><dd>{ui.info?.presentmon_version ?? "—"}</dd>
      <dt>{tk("ui.settings.privacy")}</dt><dd>{tk("ui.settings.privacy_text")}</dd>
    </dl>
  </Card>
</Page>

{#if confirmUpdate && ui.update?.available}
  <Modal
    title={tk("ui.update.confirm_title", { v: ui.update.available.version })}
    confirmLabel={tk("ui.update.confirm")}
    busy={installing}
    onconfirm={installUpdate}
    oncancel={() => (confirmUpdate = false)}
  >
    <p>{tk("ui.update.confirm_text")}</p>
    {#if installing}
      <div class="download" aria-live="polite">
        <div
          class="track"
          role="progressbar"
          aria-label={tk("ui.update.download_label")}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={share == null ? undefined : Math.round(share * 100)}
        >
          <div class="fill" class:indeterminate={share == null} style:transform={share == null ? undefined : `scaleX(${share})`}></div>
        </div>
        <p class="small muted">
          {#if !progress}
            {tk("ui.update.stage_connect")}
          {:else if share != null && share >= 1}
            {tk("ui.update.stage_verify")}
          {:else}
            {progress.total
              ? tk("ui.update.progress_pct", {
                  done: mb(progress.downloaded),
                  total: mb(progress.total),
                  pct: Math.round((share ?? 0) * 100),
                })
              : tk("ui.update.progress_unknown", { done: mb(progress.downloaded) })}
            {#if speed != null}· {tk("ui.update.speed", { v: speed.toFixed(1) })}{/if}
          {/if}
        </p>
      </div>
    {/if}
  </Modal>
{/if}

<style>
  .download {
    margin-top: calc(var(--space) * 4);
  }
  .track {
    height: 6px;
    margin-bottom: calc(var(--space) * 2);
    border-radius: 999px;
    background: var(--bg-input);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    transform-origin: left;
    background: var(--accent-gradient);
    transition: transform var(--dur) var(--ease);
  }
  /* Розмір невідомий (або ще з'єднуємось) — смужка, що біжить. */
  .fill.indeterminate {
    width: 30%;
    animation: run 1.2s var(--ease) infinite;
  }
  @keyframes run {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .fill {
      transition: none;
    }
    .fill.indeterminate {
      animation: none;
      width: 100%;
      opacity: 0.4;
    }
  }
  .rows {
    display: grid;
    gap: calc(var(--space) * 5);
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: calc(var(--space) * 4);
  }
  .kv {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 2) calc(var(--space) * 6);
    margin: 0;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
  }
  p {
    margin: 0;
  }
  .list {
    margin: 0;
    padding-left: calc(var(--space) * 5);
    display: grid;
    gap: calc(var(--space) * 1);
  }
  .actions {
    display: flex;
    gap: calc(var(--space) * 2);
  }
  .path {
    overflow-wrap: anywhere;
  }
  .error {
    color: var(--danger);
  }
  .link {
    margin-top: calc(var(--space) * 2);
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-from);
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
  .link:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
    border-radius: 4px;
  }
</style>
