<script lang="ts">
  import type { RecordProgress } from "../lib/bindings/RecordProgress";
  import { tk } from "../lib/i18n";

  // Стан запису: чекаємо гру / клавішу, прогрів, запис, пауза. Смуга — лише зарахований час.
  let { progress, game }: { progress: RecordProgress | null; game: string } = $props();

  const phase = $derived(progress?.phase ?? "waiting_for_game");
  const share = $derived(progress && progress.target_s > 0 ? Math.min(1, progress.counted_s / progress.target_s) : 0);
</script>

<div class="status {phase}" role="status" aria-live="polite">
  <div class="row">
    <span class="dot" aria-hidden="true"></span>
    <strong>{tk(`ui.rec.phase.${phase}`, { game })}</strong>
    {#if progress && (phase === "recording" || phase === "paused" || progress.counted_s > 0)}
      <span class="mono muted">{tk("ui.rec.counted", { s: Math.floor(progress.counted_s), total: Math.round(progress.target_s) })}</span>
    {/if}
  </div>
  <div
    class="track"
    role="progressbar"
    aria-valuemin={0}
    aria-valuemax={progress?.target_s ?? 0}
    aria-valuenow={progress?.counted_s ?? 0}
  >
    <div class="fill" style:transform="scaleX({share})"></div>
  </div>
  <p class="small muted">{tk(`ui.rec.hint.${phase}`, { game })}</p>
</div>

<style>
  .status {
    display: grid;
    gap: calc(var(--space) * 2.5);
  }
  .row {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
  .row .mono {
    margin-left: auto;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .recording .dot {
    background: var(--danger);
    box-shadow: 0 0 10px var(--danger);
    animation: blink 1.2s var(--ease) infinite;
  }
  .warmup .dot {
    background: var(--warn);
  }
  .paused .dot {
    background: var(--warn);
  }
  .waiting_for_game .dot,
  .waiting_for_hotkey .dot {
    background: var(--info);
  }
  .track {
    height: 6px;
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
  @keyframes blink {
    50% {
      opacity: 0.35;
    }
  }
</style>
