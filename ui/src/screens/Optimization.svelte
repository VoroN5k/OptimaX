<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { AbFinal } from "../lib/bindings/AbFinal";
  import type { AbView } from "../lib/bindings/AbView";
  import type { PowerSchemes } from "../lib/bindings/PowerSchemes";
  import type { RecordProgress } from "../lib/bindings/RecordProgress";
  import type { UiDesired } from "../lib/bindings/UiDesired";
  import { tk } from "../lib/i18n";
  import { flashBg, logLocal, ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import ChangePreview from "../components/ChangePreview.svelte";
  import Modal from "../components/Modal.svelte";
  import Page from "../components/Page.svelte";
  import ProcessPicker from "../components/ProcessPicker.svelte";
  import RecordStatus from "../components/RecordStatus.svelte";
  import StartModePicker from "../components/StartModePicker.svelte";
  import Segmented from "../components/Segmented.svelte";
  import TweakApplyModal from "../components/TweakApplyModal.svelte";

  const BALANCED = "381b4222-f694-41f0-9685-ff5bb260df2e";
  const HIGH = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";

  type Choice = { id: string; title: string; text: string; desired: UiDesired; current?: boolean };

  let mode = $state<"wizard" | "expert">("wizard");
  let schemes = $state<PowerSchemes | null>(null);
  let step = $state(1);
  let choice = $state<Choice | null>(null);
  let game = $state("");
  let runs = $state(3);
  let seconds = $state(60);
  let ab = $state<AbView | null>(null);
  let running = $state(false);
  let final = $state<AbFinal | null>(null);
  let error = $state<string | null>(null);
  let restoreReason = $state<string | null>(null);
  let expertDesired = $state<UiDesired | null>(null);
  let runProgress = $state<RecordProgress | null>(null);

  onMount(() => {
    void (async () => {
      schemes = await api.powerSchemes().catch(() => null);
      ab = await api.abState().catch(() => null);
      if (ab) step = 3;
    })();
    const sub = on("ab-progress", (p) => (runProgress = p));
    return () => void sub.then((u) => u());
  });

  const choices = $derived<Choice[]>([
    {
      id: "power-balanced",
      title: tk("ui.opt.choice.power_balanced"),
      text: tk("ui.opt.choice.power_balanced_text"),
      desired: { tweak: "power_scheme", value: BALANCED },
      current: schemes?.active === BALANCED,
    },
    {
      id: "power-high",
      title: tk("ui.opt.choice.power_high"),
      text: tk("ui.opt.choice.power_high_text"),
      desired: { tweak: "power_scheme", value: HIGH },
      current: schemes?.active === HIGH,
    },
    {
      id: "gamemode-on",
      title: tk("ui.opt.choice.gamemode_on"),
      text: tk("ui.opt.choice.gamemode_on_text"),
      desired: { tweak: "game_mode", value: true },
    },
    {
      id: "gamemode-off",
      title: tk("ui.opt.choice.gamemode_off"),
      text: tk("ui.opt.choice.gamemode_off_text"),
      desired: { tweak: "game_mode", value: false },
    },
  ]);

  async function startAb() {
    if (!choice) return;
    error = null;
    try {
      ab = await api.abStart(game.trim(), choice.desired, runs, seconds);
      step = 3;
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  async function runNext(allow = false) {
    error = null;
    running = true;
    runProgress = null;
    ui.measuring = true;
    try {
      ab = await api.abRunNext(allow);
      restoreReason = null;
    } catch (e) {
      const err = asCmdError(e);
      if (err.kind === "restore_point") restoreReason = err.message;
      else if (err.kind === "not_started") error = tk(`ui.rec.not_started.${err.message || "wait_timeout"}`);
      else {
        error = err.message;
        flashBg("warn");
      }
    } finally {
      running = false;
      ui.measuring = false;
    }
  }

  async function finish(keep: boolean) {
    try {
      final = await api.abFinish(keep);
      flashBg(final.report.effect === "better" ? "ok" : "warn");
      logLocal("info", tk(final.kept ? "ui.opt.kept_log" : "ui.opt.reverted_log"));
      ab = null;
      step = 4;
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  async function cancelAb() {
    await api.abCancel().catch(() => undefined);
    ab = null;
    step = 1;
  }

  function restart() {
    final = null;
    choice = null;
    step = 1;
  }

  const done = $derived(ab ? ab.next >= ab.order.length : false);
  const nextArm = $derived(ab && !done ? ab.order[ab.next] : null);
</script>

<Page title={tk("ui.opt.title")} subtitle={tk("ui.opt.subtitle")}>
  <div class="mode">
    <Segmented
      label={tk("ui.opt.mode")}
      bind:value={mode}
      options={[
        { value: "wizard", label: tk("ui.opt.wizard") },
        { value: "expert", label: tk("ui.opt.expert") },
      ]}
    />
  </div>

  {#if mode === "wizard"}
    <div class="steps" aria-label={tk("ui.opt.step", { n: Math.min(step, 3), m: 3 })}>
      <span class="small muted">{tk("ui.opt.step", { n: Math.min(step, 3), m: 3 })}</span>
      <div class="track"><div class="fill" style:transform="scaleX({Math.min(step, 3) / 3})"></div></div>
    </div>

    {#if step === 1}
      <Card title={tk("ui.opt.step1")} subtitle={tk("ui.opt.step1_hint")}>
        <div class="radios" role="radiogroup" aria-label={tk("ui.opt.step1")}>
          {#each choices as c (c.id)}
            <button
              type="button"
              role="radio"
              aria-checked={choice?.id === c.id}
              class="radio-card"
              class:selected={choice?.id === c.id}
              disabled={c.current}
              onclick={() => (choice = c)}
            >
              <span class="radio" aria-hidden="true"></span>
              <span
                ><strong>{c.title}</strong><span class="small muted">{c.current ? tk("ui.opt.choice.current") : c.text}</span></span
              >
            </button>
          {/each}
        </div>
      </Card>
    {:else if step === 2}
      <Card title={tk("ui.opt.step2")} subtitle={tk("ui.opt.step2_hint")}>
        <div class="grid2">
          <ProcessPicker bind:value={game} label={tk("ui.diag.game")} />
          <div class="opts">
            <span class="small muted">{tk("ui.opt.runs")}</span>
            <Segmented label={tk("ui.opt.runs")} bind:value={runs} options={[{ value: 3, label: "3 + 3" }, { value: 5, label: "5 + 5" }]} />
            <span class="small muted">{tk("ui.diag.duration")}</span>
            <Segmented
              label={tk("ui.diag.duration")}
              bind:value={seconds}
              options={[
                { value: 30, label: "30 с" },
                { value: 60, label: "60 с" },
                { value: 120, label: "120 с" },
              ]}
            />
          </div>
        </div>
        <div class="hint"><StartModePicker /></div>
        <p class="small dim hint">{tk("ui.opt.ab_rules")}</p>
      </Card>
    {:else if step === 3 && ab}
      <Card title={tk("ui.opt.step3", { game: ab.game })} subtitle={tk("ui.opt.step3_hint", { s: ab.seconds })}>
        <ChangePreview plan={ab.plan} open={false} />
        <ol class="runs">
          {#each ab.order as arm, i (i)}
            {@const r = ab.results[i]}
            <li class:current={i === ab.next} class:done={!!r}>
              <span class="arm {arm}">{arm}</span>
              <span>{tk(arm === "A" ? "ui.opt.arm_a" : "ui.opt.arm_b")}</span>
              <span class="mono">{r ? `${r.avg_fps.toFixed(1)} FPS · 1% ${r.low_1pct_fps.toFixed(1)}` : i === ab.next && running ? tk("ui.opt.recording") : "—"}</span>
              {#if r?.conditions.frame_capped}<span class="cap small">{tk("ui.opt.capped")}</span>{/if}
            </li>
          {/each}
        </ol>
        {#if running}
          <div class="run-status"><RecordStatus progress={runProgress} game={ab.game} /></div>
        {/if}
        {#if nextArm && ab.plan.info.restart_game_between_runs}
          <p class="warn small">{tk("ui.opt.restart_game")}</p>
        {/if}
      </Card>
    {:else if step === 4 && final}
      <Card title={tk("ui.opt.report")}>
        <div class="verdict {final.report.effect}">
          {tk(final.report.effect === "better" ? "ui.opt.effect_better" : final.report.effect === "worse" ? "ui.opt.effect_worse" : "ui.opt.effect_none")}
          — {tk(final.kept ? "ui.opt.kept" : "ui.opt.reverted")}
        </div>
        <pre class="md mono">{final.markdown}</pre>
        <Button small icon="copy" onclick={() => navigator.clipboard.writeText(final?.markdown ?? "")}>{tk("ui.opt.copy")}</Button>
      </Card>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  {:else}
    <Card title={tk("ui.opt.expert_title")} subtitle={tk("ui.opt.expert_hint")}>
      <div class="expert">
        <div class="row">
          <div>
            <strong>{tk("ui.opt.power")}</strong>
            <p class="small muted">
              {tk("ui.opt.power_now", {
                name: schemes?.schemes.find((s) => s.guid === schemes?.active)?.name ?? schemes?.active ?? "…",
              })}
            </p>
          </div>
          <div class="btns">
            {#each schemes?.schemes ?? [] as s (s.guid)}
              <Button small disabled={s.guid === schemes?.active} onclick={() => (expertDesired = { tweak: "power_scheme", value: s.guid })}>
                {s.name ?? s.guid}
              </Button>
            {/each}
          </div>
        </div>
        <div class="row">
          <div>
            <strong>{tk("ui.opt.gamemode")}</strong>
            <p class="small muted">{tk("ui.opt.gamemode_hint")}</p>
          </div>
          <div class="btns">
            <Button small onclick={() => (expertDesired = { tweak: "game_mode", value: true })}>{tk("ui.common.enable")}</Button>
            <Button small onclick={() => (expertDesired = { tweak: "game_mode", value: false })}>{tk("ui.common.disable")}</Button>
          </div>
        </div>
      </div>
    </Card>
  {/if}

  {#snippet secondary()}
    {#if mode === "wizard"}
      {#if step === 2}<Button variant="ghost" onclick={() => (step = 1)}>{tk("ui.common.back")}</Button>{/if}
      {#if step === 3}<Button variant="danger" icon="close" disabled={running} onclick={cancelAb}>{tk("ui.opt.cancel_ab")}</Button>{/if}
    {/if}
  {/snippet}
  {#snippet primary()}
    {#if mode === "wizard"}
      {#if step === 1}
        <Button variant="primary" disabled={!choice} onclick={() => (step = 2)}>{tk("ui.common.next")}</Button>
      {:else if step === 2}
        <Button variant="primary" icon="play" disabled={!game.trim()} onclick={startAb}>{tk("ui.opt.start_ab")}</Button>
      {:else if step === 3 && ab}
        {#if done}
          <Button variant="secondary" onclick={() => finish(false)}>{tk("ui.opt.finish_revert")}</Button>
          <Button variant="primary" icon="check" onclick={() => finish(true)}>{tk("ui.opt.finish")}</Button>
        {:else}
          <Button variant="primary" icon="play" loading={running} onclick={() => runNext(false)}>
            {tk("ui.opt.run_next", { n: ab.next + 1, m: ab.order.length, arm: nextArm ?? "" })}
          </Button>
        {/if}
      {:else if step === 4}
        <Button variant="primary" onclick={restart}>{tk("ui.opt.new_test")}</Button>
      {/if}
    {/if}
  {/snippet}
</Page>

{#if restoreReason !== null}
  <Modal
    title={tk("ui.restore.title")}
    confirmLabel={tk("ui.restore.continue")}
    danger
    busy={running}
    onconfirm={() => runNext(true)}
    oncancel={() => (restoreReason = null)}
  >
    <p>{tk("ui.restore.text", { reason: restoreReason })}</p>
  </Modal>
{/if}

{#if expertDesired}
  <TweakApplyModal
    desired={expertDesired}
    onclose={async () => {
      expertDesired = null;
      schemes = await api.powerSchemes().catch(() => schemes);
    }}
  />
{/if}

<style>
  .mode {
    display: flex;
  }
  .steps {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .track {
    height: 4px;
    border-radius: 999px;
    background: var(--bg-input);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent-gradient);
    transform-origin: left;
    transition: transform var(--dur-slow) var(--ease);
  }
  .radios {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: calc(var(--space) * 3);
  }
  .radio-card {
    display: flex;
    gap: calc(var(--space) * 3);
    align-items: flex-start;
    padding: calc(var(--space) * 4);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-input);
    cursor: pointer;
    text-align: left;
    transition: border-color var(--dur) var(--ease), box-shadow var(--dur) var(--ease);
  }
  .radio-card > span:last-child {
    display: grid;
    gap: 2px;
  }
  .radio-card:hover:not(:disabled) {
    border-color: var(--border-strong);
  }
  .radio-card:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .radio-card.selected {
    border-color: var(--accent-from);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }
  .radio {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    margin-top: 2px;
    border-radius: 50%;
    border: 2px solid var(--border-strong);
  }
  .selected .radio {
    border: 5px solid var(--accent-from);
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: calc(var(--space) * 6);
  }
  .opts {
    display: grid;
    gap: calc(var(--space) * 2);
    align-content: start;
  }
  .hint {
    margin-top: calc(var(--space) * 4);
  }
  .run-status {
    margin-top: calc(var(--space) * 4);
    padding: calc(var(--space) * 4);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-input);
  }
  .runs {
    list-style: none;
    margin: calc(var(--space) * 4) 0 0;
    padding: 0;
    display: grid;
    gap: calc(var(--space) * 1.5);
  }
  .runs li {
    display: grid;
    grid-template-columns: 28px 160px 1fr auto;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 2) calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .runs li.current {
    border-color: var(--accent-from);
    color: var(--text);
  }
  .runs li.done {
    color: var(--text);
  }
  .arm {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    font-weight: 700;
    font-size: 12px;
    background: var(--bg-input);
  }
  .arm.B {
    background: var(--accent-gradient);
    color: var(--on-accent);
  }
  .cap,
  .warn {
    color: var(--warn);
  }
  .verdict {
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border-radius: var(--radius-md);
    margin-bottom: calc(var(--space) * 3);
    border: 1px solid var(--border);
    font-weight: 600;
  }
  .verdict.better {
    color: var(--ok);
    border-color: var(--ok);
  }
  .verdict.worse {
    color: var(--danger);
    border-color: var(--danger);
  }
  .md {
    white-space: pre-wrap;
    padding: calc(var(--space) * 4);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    border: 1px solid var(--border);
    margin: 0 0 calc(var(--space) * 3);
  }
  .expert {
    display: grid;
    gap: calc(var(--space) * 4);
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: calc(var(--space) * 4);
    padding-bottom: calc(var(--space) * 4);
    border-bottom: 1px solid var(--border);
  }
  .row:last-child {
    border-bottom: none;
    padding-bottom: 0;
  }
  .btns {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 2);
    justify-content: flex-end;
  }
  .error {
    color: var(--danger);
  }
</style>
