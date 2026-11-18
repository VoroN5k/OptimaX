<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Finding } from "../lib/bindings/Finding";
  import type { Story } from "../lib/bindings/Story";
  import type { Verdict } from "../lib/bindings/Verdict";
  import { t, tk } from "../lib/i18n";
  import CheckRow from "./CheckRow.svelte";
  import Disclosure from "./Disclosure.svelte";
  import HeroCard from "./HeroCard.svelte";
  import StatusPill from "./StatusPill.svelte";

  // Розповідь простими словами зверху результату: «Як іде гра / Чому були ривки / Що зробити».
  // Тексти й вибір дій — бекенд (`diagnosis::story`); тут лише показ. Порада й кнопка дії беруться
  // з самої знахідки (за індексом у `verdicts`), тож не дублюються.
  let {
    story,
    verdicts,
    action,
    meta,
  }: {
    story: Story;
    verdicts: Verdict[];
    /** Кнопка наступного кроку знахідки (A/B, апскейл) — та сама, що в «Детальніше». */
    action?: Snippet<[Finding]>;
    /** Рядок під карткою: зарахований час, причина завершення. */
    meta?: Snippet;
  } = $props();

  const pick = (i: number | null): Finding | null => (i == null ? null : (verdicts[i]?.finding ?? null));
  const main = $derived(pick(story.action));
  const others = $derived(story.others.map(pick).filter((f): f is Finding => !!f?.advice));
  const tone = $derived(
    story.smoothness === "smooth"
      ? "ok"
      : story.smoothness === "isolated"
        ? "neutral"
        : "warn",
  );
  const insufficient = $derived(story.outcome === "insufficient");
  const hasWhy = $derived(story.why.length > 0 || story.why_unknown);
</script>

<section class="story" aria-label={tk("ui.story.label")} data-tour="diag-summary">
  <HeroCard>
    <div class="head">
      {#if !insufficient}
        <StatusPill {tone} icon={tone === "ok" ? "check" : undefined}>{tk(`diag.story.smooth.${story.smoothness}`)}</StatusPill>
      {/if}
      <h3 class="title">{tk(`diag.story.outcome.${story.outcome}`)}</h3>
    </div>

    <div class="block" data-tour="diag-stats">
      <div class="micro">{tk("ui.story.how")}</div>
      {#each story.how as line, i (i)}
        <p>{t(line)}</p>
      {/each}
    </div>

    {#if hasWhy}
      <div class="block">
        <div class="micro">{tk("ui.story.why")}</div>
        {#each story.why as c, i (i)}
          <p class:likely={c.likely}>{t(c.text)}</p>
        {/each}
        {#if story.why_unknown}<p>{tk("diag.story.why.unknown")}</p>{/if}
        <p class="small muted">{tk("ui.story.why_note")}</p>
      </div>
    {/if}

    {#if !insufficient}
      <div class="block">
        <div class="micro">{tk("ui.story.todo")}</div>
        {#if main?.advice}
          <ul class="checks">
            <CheckRow n={1}>
              {t(main.advice)}
              {#snippet action()}{@render actionFor(main)}{/snippet}
            </CheckRow>
          </ul>
        {:else}
          <p>{tk("ui.story.nothing")}</p>
        {/if}
        {#if others.length}
          <Disclosure title={tk("ui.story.others")} count={others.length}>
            <ul class="checks">
              {#each others as f, i (i)}
                <CheckRow n={i + (main ? 2 : 1)}>
                  {#if f.advice}{t(f.advice)}{/if}
                  {#snippet action()}{@render actionFor(f)}{/snippet}
                </CheckRow>
              {/each}
            </ul>
          </Disclosure>
        {/if}
      </div>
    {/if}

    {#if meta}<div class="meta small muted">{@render meta()}</div>{/if}
  </HeroCard>
</section>

{#snippet actionFor(f: Finding)}
  {#if action}{@render action(f)}{/if}
{/snippet}

<style>
  .story {
    scroll-margin-top: calc(var(--space) * 4);
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
  .title {
    margin: 0;
    font-size: var(--fs-section);
    font-weight: 400;
    letter-spacing: -0.02em;
    color: var(--text);
  }
  .block {
    display: grid;
    gap: calc(var(--space) * 1.5);
    margin-top: calc(var(--space) * 5);
  }
  .block p {
    margin: 0;
    color: var(--text-2);
  }
  .block p.likely {
    color: var(--text);
  }
  .checks {
    margin: 0;
    padding: 0;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 1) calc(var(--space) * 4);
    margin-top: calc(var(--space) * 5);
  }
</style>
