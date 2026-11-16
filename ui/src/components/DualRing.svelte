<script lang="ts" module>
  /** Одне кільце: відсоток (0–100) або `null` — даних немає. */
  export type RingValue = { value: number | null; label: string };
</script>

<script lang="ts">
  // Кільце-індикатор (docs/DESIGN.md розд. 7, DualRing): SVG 232×232, показ 208px; зовнішні
  // тік-позначки; зовнішнє кільце r=88 (помаранчевий градієнт, головне обмеження) і, якщо є,
  // внутрішнє r=66 (акцент, напр. GPU). Дуга — з реального відсотка; `null` — порожнє кільце й
  // «—» (нічого не вигадуємо). Над кільцем — промінь, що обертається (лише режим «Повні»).
  let {
    outer,
    inner,
    center,
    aria,
  }: {
    outer: RingValue;
    inner?: RingValue;
    /** Підпис під числом у центрі (великими літерами). */
    center: string;
    /** Повний текстовий зміст для читачів екрана. */
    aria: string;
  } = $props();

  const C_OUT = 2 * Math.PI * 88;
  const C_IN = 2 * Math.PI * 66;
  const uid = Math.random().toString(36).slice(2, 8);
  const pct = (v: number | null) => (v === null ? null : Math.max(0, Math.min(100, v)));
  const o = $derived(pct(outer.value));
  const i = $derived(inner ? pct(inner.value) : null);
  const fmt = (v: number | null) => (v === null ? "—" : String(Math.round(v)));
</script>

<div class="wrap">
  <div class="ring" role="img" aria-label={aria}>
    <div class="beam" aria-hidden="true"></div>
    <svg width="208" height="208" viewBox="0 0 232 232" aria-hidden="true">
      <defs>
        <linearGradient id="o-{uid}" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="var(--warn-from)" />
          <stop offset="1" stop-color="var(--warn-to)" />
        </linearGradient>
        <linearGradient id="i-{uid}" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="var(--a1)" />
          <stop offset="1" stop-color="var(--a2)" />
        </linearGradient>
      </defs>
      <circle class="ticks" cx="116" cy="116" r="108" fill="none" stroke="var(--ring-ticks)" stroke-width="6" stroke-dasharray="1.2 8.4" />
      <circle cx="116" cy="116" r="88" fill="none" stroke="var(--ring-track)" stroke-width="10" />
      {#if o !== null}
        <circle
          class="arc out"
          cx="116"
          cy="116"
          r="88"
          fill="none"
          stroke="url(#o-{uid})"
          stroke-width="10"
          stroke-linecap="round"
          stroke-dasharray={C_OUT}
          stroke-dashoffset={C_OUT * (1 - o / 100)}
          style:--c={C_OUT}
          transform="rotate(-90 116 116)"
        />
      {/if}
      {#if inner}
        <circle cx="116" cy="116" r="66" fill="none" stroke="var(--ring-track)" stroke-width="10" />
        {#if i !== null}
          <circle
            class="arc in"
            cx="116"
            cy="116"
            r="66"
            fill="none"
            stroke="url(#i-{uid})"
            stroke-width="10"
            stroke-linecap="round"
            stroke-dasharray={C_IN}
            stroke-dashoffset={C_IN * (1 - i / 100)}
            style:--c={C_IN}
            transform="rotate(-90 116 116)"
          />
        {/if}
      {/if}
    </svg>
    <div class="center">
      <div class="big num">{fmt(o)}{#if o !== null}<span class="pct">%</span>{/if}</div>
      <div class="caption">{center}</div>
    </div>
  </div>
  <div class="values">
    <div class="v">
      <div class="key small"><span class="sw out" aria-hidden="true"></span>{outer.label}</div>
      <div class="n num warn">{fmt(o)}{#if o !== null}&nbsp;%{/if}</div>
    </div>
    {#if inner}
      <div class="v">
        <div class="key small"><span class="sw in" aria-hidden="true"></span>{inner.label}</div>
        <div class="n num">{fmt(i)}{#if i !== null}&nbsp;%{/if}</div>
      </div>
    {/if}
  </div>
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--space) * 3.5);
  }
  .ring {
    position: relative;
    width: 208px;
    height: 208px;
    flex: 0 0 208px;
  }
  svg {
    position: absolute;
    inset: 0;
  }
  .beam {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: conic-gradient(from 0deg, transparent 0deg, transparent 290deg, rgba(var(--glow), 0.28) 360deg);
    -webkit-mask: radial-gradient(circle, transparent 0 46px, #000 47px 100px, transparent 101px);
    mask: radial-gradient(circle, transparent 0 46px, #000 47px 100px, transparent 101px);
    animation: spin 7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .out {
    filter: drop-shadow(0 0 8px rgba(var(--warn-glow), 0.55));
  }
  .in {
    filter: drop-shadow(0 0 8px rgba(var(--glow), 0.55));
  }
  /* Кільця малюються до свого значення (одноразово), тіки проявляються. */
  .arc {
    animation: draw 1.8s var(--ease) backwards;
  }
  .arc.out {
    animation-delay: 0.5s;
  }
  .arc.in {
    animation-delay: 0.7s;
  }
  @keyframes draw {
    from {
      stroke-dashoffset: var(--c);
    }
  }
  .ticks {
    animation: fade 1.2s ease 0.3s backwards;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
  }
  .big {
    font-size: 42px;
    line-height: 44px;
    font-weight: 300;
    letter-spacing: -0.04em;
  }
  .pct {
    margin-left: 2px;
    font-size: 16px;
    letter-spacing: 0;
    color: var(--text-2);
  }
  .caption {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--text-3);
  }
  .values {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: calc(var(--space) * 3) calc(var(--space) * 6.5);
  }
  .v {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .key {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    color: var(--text-2);
  }
  .sw {
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }
  .sw.out {
    background: var(--warn-gradient);
  }
  .sw.in {
    background: var(--accent-gradient);
  }
  .n {
    font-size: 24px;
    line-height: 28px;
    font-weight: 300;
    letter-spacing: -0.03em;
  }
  .n.warn {
    color: var(--warn);
  }
  :global(:root[data-effects="calm"]) .beam,
  :global(:root.measuring) .beam {
    animation: none;
    opacity: 0;
  }
  :global(:root[data-effects="off"]) .ring :is(.beam, .arc, .ticks) {
    animation: none;
  }
  :global(:root[data-effects="off"]) .beam {
    opacity: 0;
  }
  @media (prefers-reduced-motion: reduce) {
    .beam,
    .arc,
    .ticks {
      animation: none;
    }
    .beam {
      opacity: 0;
    }
  }
</style>
