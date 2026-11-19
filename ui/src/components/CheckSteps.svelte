<script lang="ts">
  import { tk } from "../lib/i18n";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";

  // Смужка кроків сценарію «Перевірити мою гру». Поточний крок — номером, текстом і
  // `aria-current`, пройдені — галочкою (не лише кольором). `final` — сценарій закінчився на
  // цьому кроці (усе гаразд): далі кроки неактивні.
  let { step, final = false, onfinish }: { step: number; final?: boolean; onfinish: () => void } = $props();

  const names = ["game", "record", "verdict", "action", "result"] as const;
</script>

<section class="bar" data-check-steps aria-label={tk("ui.check.title")}>
  <ol aria-label={tk("ui.check.steps.label")}>
    {#each names as name, i (name)}
      {@const n = i + 1}
      <li class:done={n < step || (final && n === step)} class:current={n === step} class:off={final && n > step} aria-current={n === step ? "step" : undefined}>
        <span class="n mono" aria-hidden="true">
          {#if n < step || (final && n === step)}<Icon name="check" size={12} />{:else}{n}{/if}
        </span>
        <span class="label">{tk(`ui.check.steps.${name}`)}</span>
      </li>
    {/each}
  </ol>
  <span class="small muted sr">{tk("ui.check.step_n", { n: step })}</span>
  <Button small variant="secondary" onclick={onfinish}>{tk("ui.check.finish")}</Button>
</section>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  ol {
    flex: 1;
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 2) calc(var(--space) * 4);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    color: var(--text-muted);
    font-size: 13px;
  }
  li.current {
    color: var(--text);
    font-weight: 600;
  }
  li.off {
    opacity: 0.5;
  }
  .n {
    display: inline-grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 1px solid var(--border-strong);
    font-size: 11px;
  }
  li.current .n {
    border-color: var(--accent-from);
  }
  li.done .n {
    border-color: var(--ok);
    color: var(--ok);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
