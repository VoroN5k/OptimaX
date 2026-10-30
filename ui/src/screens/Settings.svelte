<script lang="ts">
  import { api, asCmdError } from "../lib/api";
  import type { AnimationLevel } from "../lib/bindings/AnimationLevel";
  import type { DotsStats } from "../lib/dots";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import Page from "../components/Page.svelte";
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
      {#if dotsStats}
        <p class="small dim mono">
          {tk("ui.settings.anim_stats", { dots: dotsStats.dots, ms: dotsStats.avgFrameMs.toFixed(2), fps: dotsStats.fps.toFixed(0) })}
        </p>
      {/if}
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

<style>
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
</style>
