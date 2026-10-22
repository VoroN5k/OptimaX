<script lang="ts">
  import { api, asCmdError } from "../lib/api";
  import type { AuditReport } from "../lib/bindings/AuditReport";
  import type { Finding } from "../lib/bindings/Finding";
  import type { UiDesired } from "../lib/bindings/UiDesired";
  import { t, tk } from "../lib/i18n";
  import { flashBg, ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import FindingCard from "../components/FindingCard.svelte";
  import Page from "../components/Page.svelte";
  import Ring from "../components/Ring.svelte";
  import StateView from "../components/StateView.svelte";
  import TweakApplyModal from "../components/TweakApplyModal.svelte";

  const BALANCED = "381b4222-f694-41f0-9685-ff5bb260df2e";

  let report = $state<AuditReport | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let fix = $state<UiDesired | null>(null);

  const counts = $derived({
    high: report?.findings.filter((f) => f.impact === "high").length ?? 0,
    moderate: report?.findings.filter((f) => f.impact === "moderate").length ?? 0,
    info: report?.findings.filter((f) => f.impact === "info").length ?? 0,
  });
  const okShare = $derived(
    report && report.findings.length ? (counts.info / report.findings.length) * 100 : 0,
  );

  /** Знахідки, які OptimaX уміє виправити сам (через перегляд змін і журнал). */
  function fixFor(f: Finding): UiDesired | null {
    if (f.id === "power.saver") return { tweak: "power_scheme", value: BALANCED };
    if (f.id === "os.game_mode_off") return { tweak: "game_mode", value: true };
    return null;
  }

  async function run() {
    loading = true;
    error = null;
    ui.bgMode = "scanning";
    try {
      report = await api.runAudit();
      flashBg(report.findings.some((f) => f.impact !== "info") ? "warn" : "ok");
    } catch (e) {
      error = asCmdError(e).message;
      flashBg("warn");
    } finally {
      loading = false;
    }
  }
</script>

<Page title={tk("ui.analysis.title")} subtitle={tk("ui.analysis.subtitle")}>
  {#if !report && !loading && !error}
    <Card>
      <StateView kind="empty" title={tk("ui.analysis.empty_title")} text={tk("ui.analysis.empty_text")}>
        {#snippet action()}
          <Button variant="primary" icon="analysis" onclick={run}>{tk("ui.analysis.run")}</Button>
        {/snippet}
      </StateView>
    </Card>
  {:else if loading && !report}
    <Card><StateView kind="loading" title={tk("ui.analysis.loading")} /></Card>
  {:else if error}
    <Card>
      <StateView kind="error" title={tk("ui.common.error")} text={error}>
        {#snippet action()}<Button onclick={run}>{tk("ui.common.retry")}</Button>{/snippet}
      </StateView>
    </Card>
  {/if}

  {#if report}
    <Card>
      <div class="summary">
        <Ring value={okShare} label={tk("ui.analysis.ring_label")} caption={tk("ui.analysis.ring_caption")} />
        <div class="counts">
          <div class="count high"><strong>{counts.high}</strong><span>{tk("ui.badge.high")}</span></div>
          <div class="count moderate"><strong>{counts.moderate}</strong><span>{tk("ui.badge.moderate")}</span></div>
          <div class="count info"><strong>{counts.info}</strong><span>{tk("ui.badge.info")}</span></div>
        </div>
        <p class="verdict">
          {counts.high + counts.moderate === 0 ? tk("ui.analysis.nothing_to_fix") : tk("ui.analysis.has_findings")}
        </p>
      </div>
    </Card>

    <div class="list">
      {#each report.findings as f (f.id + t(f.title))}
        {@const desired = fixFor(f)}
        <FindingCard finding={f}>
          {#snippet action()}
            {#if desired}
              <Button small variant="primary" icon="check" onclick={() => (fix = desired)}>{tk("ui.analysis.fix")}</Button>
            {/if}
          {/snippet}
        </FindingCard>
      {/each}
    </div>

    {#if report.skipped.length}
      <Card title={tk("ui.analysis.skipped")}>
        <ul class="skipped">
          {#each report.skipped as s (s.check)}
            <li><strong>{t({ key: `audit.check.${s.check}`, args: {} })}</strong> — <span class="muted">{s.error}</span></li>
          {/each}
        </ul>
      </Card>
    {/if}
  {/if}

  {#snippet primary()}
    {#if report}
      <Button variant="primary" icon="analysis" {loading} onclick={run}>{tk("ui.analysis.rerun")}</Button>
    {/if}
  {/snippet}
</Page>

{#if fix}
  <TweakApplyModal
    desired={fix}
    onclose={(applied) => {
      fix = null;
      if (applied) void run();
    }}
  />
{/if}

<style>
  .summary {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 8);
    flex-wrap: wrap;
  }
  .counts {
    display: flex;
    gap: calc(var(--space) * 3);
  }
  .count {
    display: grid;
    gap: 2px;
    min-width: 108px;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-input);
  }
  .count strong {
    font-size: 24px;
    font-weight: 600;
  }
  .count span {
    font-size: 12px;
    color: var(--text-muted);
  }
  .count.high strong {
    color: var(--danger);
  }
  .count.moderate strong {
    color: var(--warn);
  }
  .count.info strong {
    color: var(--info);
  }
  .verdict {
    flex: 1;
    min-width: 220px;
    color: var(--text-muted);
  }
  .list {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .skipped {
    margin: 0;
    padding-left: calc(var(--space) * 5);
    display: grid;
    gap: calc(var(--space) * 1);
  }
</style>
