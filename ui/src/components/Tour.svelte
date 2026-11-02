<script lang="ts">
  import { tick } from "svelte";
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import { STEPS, back, currentStep, finish, next, tour } from "../lib/tour.svelte";
  import Button from "./Button.svelte";

  // Знайомство: затемнення з «вікном» над справжнім елементом і бульбашка з поясненням.
  // Затемнення не перехоплює мишу — підсвічений елемент можна одразу використати
  // (вибрати гру, натиснути «Почати запис»).

  /** Скільки чекати елемент після переходу (екран монтується, блок розгортається). */
  const WAIT_MS = 1200;
  const PAD = 6;
  const GAP = 12;
  const BUBBLE_W = 380;

  const step = $derived(currentStep());
  let rect = $state<DOMRect | null>(null);
  let missing = $state(false);
  let bubble = $state<HTMLDivElement | null>(null);
  let bubbleH = $state(0);
  let vw = $state(window.innerWidth);
  let vh = $state(window.innerHeight);
  let lastDir: 1 | -1 = 1;

  const chapterSteps = $derived(step ? STEPS.filter((s) => s.chapter === step.chapter) : []);
  const posInChapter = $derived(step ? chapterSteps.indexOf(step) : 0);
  const isLast = $derived(tour.index === STEPS.length - 1);
  const paragraphs = $derived(step ? tk(`ui.tour.${step.id}.text`).split("\n") : []);

  function find(): HTMLElement | null {
    if (!step?.target) return null;
    return document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`);
  }

  function measure() {
    const el = find();
    const r = el?.getBoundingClientRect() ?? null;
    rect = r && r.width > 0 && r.height > 0 ? r : null;
    vw = window.innerWidth;
    vh = window.innerHeight;
    if (bubble) bubbleH = bubble.offsetHeight;
  }

  // Новий крок: дочекатися елемента, прокрутити до нього, сфокусувати бульбашку.
  $effect(() => {
    const s = step;
    if (!s) return;
    rect = null;
    missing = false;
    let cancelled = false;
    const started = performance.now();
    const look = async () => {
      await tick();
      if (cancelled) return;
      if (!s.target) {
        measure();
        bubble?.focus();
        return;
      }
      const el = find();
      if (el) {
        const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
        el.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
        setTimeout(() => !cancelled && measure(), reduce ? 0 : 350);
        measure();
        bubble?.focus();
        return;
      }
      if (performance.now() - started < WAIT_MS) {
        setTimeout(() => void look(), 100);
        return;
      }
      if (s.optional) {
        if (lastDir > 0) next();
        else back();
        return;
      }
      // Обов'язковий елемент так і не з'явився (напр. іде запис) — показуємо крок по центру
      // й далі зрідка перевіряємо, чи він не з'явився.
      missing = true;
      bubble?.focus();
    };
    void look();
    // Розмітка змінюється (прокрутка, розгортання блоків) — оновлюємо положення.
    const onChange = () => measure();
    window.addEventListener("resize", onChange);
    window.addEventListener("scroll", onChange, true);
    const poll = setInterval(measure, 300);
    return () => {
      cancelled = true;
      window.removeEventListener("resize", onChange);
      window.removeEventListener("scroll", onChange, true);
      clearInterval(poll);
    };
  });

  function goNext() {
    lastDir = 1;
    next();
  }
  function goBack() {
    lastDir = -1;
    back();
  }

  function onKey(e: KeyboardEvent) {
    if (!tour.active) return;
    if (e.key === "Escape") {
      e.preventDefault();
      finish();
    }
  }

  // Положення бульбашки: під елементом, якщо вміщається, інакше над ним; по центру — без елемента.
  const pos = $derived.by(() => {
    if (!rect) return null;
    const w = Math.min(BUBBLE_W, vw - 32);
    const left = Math.min(Math.max(16, rect.left + rect.width / 2 - w / 2), vw - w - 16);
    const below = rect.bottom + PAD + GAP;
    const top =
      below + bubbleH <= vh - 16 ? below : Math.max(16, rect.top - PAD - GAP - bubbleH);
    return { left, top, w };
  });
</script>

<svelte:window onkeydown={onKey} />

{#if step}
  <div class="tour" role="presentation">
    {#if rect}
      <div
        class="hole"
        style:left="{rect.left - PAD}px"
        style:top="{rect.top - PAD}px"
        style:width="{rect.width + PAD * 2}px"
        style:height="{rect.height + PAD * 2}px"
      ></div>
    {:else}
      <div class="scrim"></div>
    {/if}

    <div
      bind:this={bubble}
      class="bubble"
      class:center={!pos}
      class:wide={!step.target}
      style:left={pos ? `${pos.left}px` : null}
      style:top={pos ? `${pos.top}px` : null}
      style:width={pos ? `${pos.w}px` : null}
      role="dialog"
      aria-modal="false"
      aria-labelledby="tour-title"
      tabindex="-1"
    >
      <p class="chapter small">
        {tk("ui.tour.step", { n: step.chapter, total: 3 })} · {tk(`ui.tour.chapter.${step.chapter}`)}
      </p>
      <h2 id="tour-title">{tk(`ui.tour.${step.id}.title`)}</h2>
      <div class="text" aria-live="polite">
        {#each paragraphs as p, i (i)}
          <p>{p}</p>
        {/each}
        {#if step.id === "welcome"}
          <ul class="features">
            {#each [1, 2, 3, 4] as n (n)}
              <li>
                <strong>{tk(`ui.tour.welcome.f${n}.title`)}</strong>
                <span class="muted">{tk(`ui.tour.welcome.f${n}.text`)}</span>
              </li>
            {/each}
          </ul>
        {/if}
        {#if missing && ui.measuring}
          <p class="waiting small">{tk("ui.tour.waiting_record")}</p>
        {/if}
      </div>

      <div class="foot">
        <div class="dots" aria-hidden="true">
          {#each chapterSteps as s, i (s.id)}
            <i class:on={i <= posInChapter}></i>
          {/each}
        </div>
        <div class="buttons">
          {#if tour.index > 0}
            <Button small variant="secondary" onclick={goBack}>{tk("ui.tour.back")}</Button>
          {/if}
          <Button small variant="primary" onclick={isLast ? finish : goNext}>
            {isLast ? tk("ui.tour.finish") : tour.index === 0 ? tk("ui.tour.begin") : tk("ui.tour.next")}
          </Button>
        </div>
      </div>
      {#if !isLast}
        <button type="button" class="skip small" onclick={finish}>{tk("ui.tour.skip")}</button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .tour {
    position: fixed;
    inset: 0;
    z-index: 50;
    pointer-events: none;
  }
  .scrim {
    position: absolute;
    inset: 0;
    background: var(--scrim);
  }
  .hole {
    position: fixed;
    border-radius: var(--radius-md);
    box-shadow:
      0 0 0 2px var(--accent-from),
      0 0 24px var(--accent-glow),
      0 0 0 9999px var(--scrim);
    transition:
      left var(--dur) var(--ease),
      top var(--dur) var(--ease),
      width var(--dur) var(--ease),
      height var(--dur) var(--ease);
  }
  .bubble {
    position: fixed;
    display: grid;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 5);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
    box-shadow: var(--shadow-modal);
    pointer-events: auto;
    outline: none;
    animation: appear var(--dur) var(--ease);
  }
  .bubble.center {
    left: 50%;
    top: 50%;
    width: min(440px, calc(100vw - 32px));
    transform: translate(-50%, -50%);
    animation: none;
  }
  .bubble.center.wide {
    width: min(560px, calc(100vw - 32px));
  }
  .chapter {
    margin: 0;
    color: var(--accent-from);
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .text {
    display: grid;
    gap: calc(var(--space) * 2);
    font-size: 13px;
    line-height: 1.55;
    color: var(--text-muted);
  }
  .text p {
    margin: 0;
  }
  .features {
    display: grid;
    gap: calc(var(--space) * 3);
    margin: calc(var(--space) * 1) 0 0;
    padding: 0;
    list-style: none;
  }
  .features li {
    display: grid;
    gap: 2px;
    padding: calc(var(--space) * 3) calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  .features strong {
    color: var(--text);
    font-weight: 600;
  }
  .waiting {
    color: var(--warn);
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3);
  }
  .dots {
    display: flex;
    gap: 6px;
  }
  .dots i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--border-strong);
  }
  .dots i.on {
    background: var(--accent-from);
  }
  .buttons {
    display: flex;
    gap: calc(var(--space) * 2);
  }
  .skip {
    justify-self: end;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
  }
  .skip:hover {
    color: var(--text-muted);
  }
  .skip:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
    border-radius: 4px;
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bubble {
      animation: none;
    }
    .hole {
      transition: none;
    }
  }
</style>
