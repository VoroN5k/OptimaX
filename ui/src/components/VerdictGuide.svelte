<script lang="ts">
  import { onMount } from "svelte";
  import { tk } from "../lib/i18n";
  import { startTour } from "../lib/tour.svelte";
  import Button from "./Button.svelte";

  // Довідник: що означає кожен вердикт діагностики, що з ним робити й як читати цифри.
  let { onclose }: { onclose: () => void } = $props();

  const VERDICTS = ["gpu", "cpu", "cap", "stutter", "unclear", "insufficient"] as const;
  const TERMS = ["avg", "low1", "low01", "p99", "confidence", "badge"] as const;

  let dialog = $state<HTMLDivElement | null>(null);
  onMount(() => dialog?.focus());

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    }
  }

  function replay() {
    onclose();
    startTour();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div bind:this={dialog} class="guide" role="dialog" aria-modal="true" aria-labelledby="guide-title" tabindex="-1">
  <header>
    <h2 id="guide-title">{tk("ui.guide.title")}</h2>
    <p class="muted small">{tk("ui.guide.subtitle")}</p>
  </header>

  <div class="scroll">
    <h3 class="section">{tk("ui.guide.verdicts")}</h3>
    <div class="verdicts">
      {#each VERDICTS as v (v)}
        <article>
          <h4>{tk(`ui.guide.v.${v}.title`)}</h4>
          <p>{tk(`ui.guide.v.${v}.means`)}</p>
          <p class="todo"><span aria-hidden="true">→</span> {tk(`ui.guide.v.${v}.todo`)}</p>
        </article>
      {/each}
    </div>

    <h3 class="section">{tk("ui.guide.terms")}</h3>
    <dl class="terms">
      {#each TERMS as term (term)}
        <dt>{tk(`ui.guide.t.${term}.title`)}</dt>
        <dd>{tk(`ui.guide.t.${term}.text`)}</dd>
      {/each}
    </dl>
  </div>

  <footer>
    <Button variant="secondary" onclick={replay}>{tk("ui.guide.replay")}</Button>
    <Button variant="primary" onclick={onclose}>{tk("ui.guide.close")}</Button>
  </footer>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--scrim);
  }
  .guide {
    position: fixed;
    z-index: 41;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    display: grid;
    grid-template-rows: auto 1fr auto;
    width: min(720px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
    box-shadow: var(--shadow-modal);
    outline: none;
  }
  header {
    display: grid;
    gap: calc(var(--space) * 1);
    padding: calc(var(--space) * 5) calc(var(--space) * 6) calc(var(--space) * 3);
  }
  h2 {
    font-size: 20px;
    font-weight: 600;
  }
  .scroll {
    overflow-y: auto;
    padding: 0 calc(var(--space) * 6) calc(var(--space) * 4);
  }
  .section {
    margin: calc(var(--space) * 3) 0 calc(var(--space) * 2);
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .verdicts {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  article {
    display: grid;
    gap: calc(var(--space) * 1);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    font-size: 13px;
  }
  h4 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  article p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  article .todo {
    color: var(--text);
  }
  .todo span {
    color: var(--accent-from);
  }
  .terms {
    display: grid;
    grid-template-columns: minmax(140px, max-content) 1fr;
    gap: calc(var(--space) * 2) calc(var(--space) * 4);
    margin: 0;
    font-size: 13px;
  }
  dt {
    font-weight: 600;
  }
  dd {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
    padding: calc(var(--space) * 4) calc(var(--space) * 6);
    border-top: 1px solid var(--border);
  }
</style>
