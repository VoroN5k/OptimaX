<script lang="ts">
  import { api, asCmdError } from "../lib/api";
  import type { TimingSummary } from "../lib/bindings/TimingSummary";
  import { t, tk } from "../lib/i18n";
  import { ui, up } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import Page from "../components/Page.svelte";
  import StateView from "../components/StateView.svelte";
  import Toggle from "../components/Toggle.svelte";
  import UpscaleOptions from "../components/UpscaleOptions.svelte";

  /** Час перейти в гру після натискання кнопки. */
  const DELAY_S = 5;

  let error = $state<string | null>(null);

  const view = $derived(up.view);
  const running = $derived(view?.running ?? null);
  const metrics = $derived(running?.stats.metrics ?? null);

  async function save() {
    await api.setSettings($state.snapshot(ui.settings));
  }

  async function call(f: () => Promise<void>) {
    error = null;
    try {
      await f();
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  const pair = (s: TimingSummary | null | undefined) =>
    s ? tk("ui.up.ms_pair", { m: s.median.toFixed(2), p: s.p95.toFixed(2) }) : "—";
</script>

<Page title={tk("ui.up.title")} subtitle={tk("ui.up.subtitle")}>
  <Card>
    {#if running}
      <div class="status">
        <span class="dot on" aria-hidden="true"></span>
        <div>
          <h2>{tk("ui.up.running", { game: running.title || running.exe, scale: running.scale.toFixed(2) })}</h2>
          <p class="small muted">
            <!-- Фактичний фільтр відомий після першого кадру (NIS поза ×1…×2 стає білінійним). -->
            {tk(`upscale.filter.${running.stats.presented > 0 ? running.stats.filter : running.settings.filter}`)}
            {#if running.stats.partial} · {tk("ui.up.partial")}{/if}
            {#if running.from_profile} · {tk("ui.up.from_profile")}{/if}
            {#if !running.stats.visible} · {tk("ui.up.hidden")}{/if}
          </p>
        </div>
      </div>
      {#if metrics}
        <div class="metrics" aria-live="polite">
          <div><strong>{pair(metrics.gpu_ms)}</strong><span>{tk("ui.up.m.gpu")}</span></div>
          <div><strong>{pair(metrics.latency_ms)}</strong><span>{tk("ui.up.m.processing")}</span></div>
          <div>
            <strong>{pair(metrics.display_ms)}</strong>
            <span>
              {tk("ui.up.m.latency")}
              {#if metrics.late_percent != null} · {tk("ui.up.m.late", { v: metrics.late_percent.toFixed(1) })}{/if}
            </span>
          </div>
          <div><strong>{metrics.cpu_percent.toFixed(1)}%</strong><span>{tk("ui.up.m.cpu")}</span></div>
        </div>
        <p class="small dim">{tk("ui.up.m.note")}</p>
      {/if}
    {:else if view?.countdown}
      <div class="status">
        <span class="dot wait" aria-hidden="true"></span>
        <div>
          <h2>{tk("ui.up.countdown", { s: view.countdown })}</h2>
          <p class="small muted">{tk("ui.up.countdown_hint")}</p>
        </div>
      </div>
    {:else}
      <StateView kind="empty" title={tk("ui.up.idle_title")} text={tk("ui.up.idle_text")} />
    {/if}
    {#if view?.notice && !running}
      <p class="notice small">{t(view.notice)}</p>
    {/if}
    {#if error}<p class="notice small">{error}</p>{/if}
  </Card>

  <Card title={tk("ui.up.settings")} subtitle={tk("ui.up.settings_hint")}>
    {#if ui.settings.upscale}
      <UpscaleOptions bind:settings={ui.settings.upscale} onchange={save} />
    {/if}
  </Card>

  <Card title={tk("ui.up.how")}>
    <ul class="how">
      <li>{tk("ui.up.how.windowed")}</li>
      <li>{tk("ui.up.how.start")}</li>
      <li>{tk("ui.up.how.exclusive")}</li>
      <li>{tk("ui.up.how.profile")}</li>
    </ul>
    <div class="hotkey">
      <div>
        <strong>{tk("ui.up.hotkey")}</strong>
        <p class="small muted">
          {#if view?.hotkey_error}{t(view.hotkey_error)}{:else}{tk("ui.up.hotkey_hint")}{/if}
        </p>
      </div>
      <Toggle label={tk("ui.up.hotkey")} bind:checked={ui.settings.upscale_hotkey} onchange={() => save()} />
    </div>
  </Card>

  {#snippet secondary()}
    {#if view?.countdown}
      <Button variant="secondary" icon="close" onclick={() => call(api.upscaleCancel)}>{tk("ui.up.cancel")}</Button>
    {:else if running}
      <Button variant="secondary" icon="stop" onclick={() => call(api.upscaleStop)}>{tk("ui.up.stop")}</Button>
    {/if}
  {/snippet}
  {#snippet primary()}
    <Button
      variant="primary"
      icon="play"
      disabled={!!running || !!view?.countdown}
      onclick={() => call(() => api.upscaleStart(DELAY_S))}
    >
      {tk("ui.up.start", { s: DELAY_S })}
    </Button>
  {/snippet}
</Page>

<style>
  .status {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 4);
  }
  .status h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .status p {
    margin: calc(var(--space) * 1) 0 0;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .dot.on {
    background: var(--ok);
    box-shadow: 0 0 10px var(--ok);
  }
  .dot.wait {
    background: var(--warn);
  }
  .metrics {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: calc(var(--space) * 3);
    margin: calc(var(--space) * 5) 0 calc(var(--space) * 3);
  }
  .metrics div {
    display: grid;
    gap: calc(var(--space) * 1);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    border: 1px solid var(--border);
  }
  .metrics strong {
    font-family: "JetBrains Mono", monospace;
    font-size: 16px;
    font-weight: 600;
  }
  .metrics span {
    font-size: 12px;
    color: var(--text-muted);
  }
  .notice {
    margin: calc(var(--space) * 3) 0 0;
    color: var(--warn);
  }
  .how {
    margin: 0 0 calc(var(--space) * 5);
    padding-left: calc(var(--space) * 5);
    display: grid;
    gap: calc(var(--space) * 2);
    color: var(--text-muted);
  }
  .hotkey {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: calc(var(--space) * 4);
  }
  .hotkey p {
    margin: calc(var(--space) * 1) 0 0;
  }
</style>
