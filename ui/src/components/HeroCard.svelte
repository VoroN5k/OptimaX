<script lang="ts">
  import type { Snippet } from "svelte";

  // Головна картка (docs/DESIGN.md розд. 7, HeroCard): рамка 1px, по якій обертається світло
  // (conic-gradient, 9 с), усередині — картка радіусом 19 з відступом 28 і двома розмитими
  // плямами (акцентна згори зліва, помаранчева знизу справа). Нескінченний рух — лише в режимі
  // «Повні» й не під час вимірювань (керують атрибути на :root).
  let { label, children }: { label?: string; children: Snippet } = $props();
</script>

<article class="frame" aria-label={label}>
  <div class="spin ambient" aria-hidden="true"></div>
  <div class="inner">
    <div class="blobs" aria-hidden="true">
      <div class="blob a ambient"></div>
      <div class="blob b ambient"></div>
    </div>
    <div class="content">{@render children()}</div>
  </div>
</article>

<style>
  .frame {
    position: relative;
    overflow: hidden;
    padding: 1px;
    border-radius: var(--radius-hero);
    background: var(--line);
    box-shadow: var(--shadow-hero);
  }
  .spin {
    position: absolute;
    left: -50%;
    top: -50%;
    width: 200%;
    height: 200%;
    background: conic-gradient(
      from 0deg,
      transparent 0deg,
      transparent 250deg,
      rgba(var(--glow), 0) 270deg,
      var(--a1) 330deg,
      transparent 360deg
    );
    will-change: transform;
    animation: spin 9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .inner {
    position: relative;
    overflow: hidden;
    border-radius: calc(var(--radius-hero) - 1px);
    background: var(--hero);
    box-shadow: inset 0 1px 0 var(--fill-3);
    padding: calc(var(--space) * 7);
  }
  .blobs {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .blob {
    position: absolute;
    border-radius: 50%;
    will-change: transform;
  }
  /* Готові радіальні градієнти замість filter: blur — той самий вигляд, дешевше для WebView2. */
  .a {
    width: 520px;
    height: 400px;
    left: -140px;
    top: -180px;
    background: radial-gradient(closest-side, rgba(var(--glow), 0.2), transparent);
    animation: drift-a 18s ease-in-out infinite alternate;
  }
  .b {
    width: 460px;
    height: 360px;
    right: -120px;
    bottom: -190px;
    background: radial-gradient(closest-side, rgba(var(--warn-glow), 0.14), transparent);
    animation: drift-b 22s ease-in-out infinite alternate;
  }
  @keyframes drift-a {
    from {
      transform: translate3d(-6%, -4%, 0) scale(1);
    }
    to {
      transform: translate3d(8%, 6%, 0) scale(1.12);
    }
  }
  @keyframes drift-b {
    from {
      transform: translate3d(6%, 4%, 0) scale(1.1);
    }
    to {
      transform: translate3d(-8%, -6%, 0) scale(1);
    }
  }
  .content {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: calc(var(--space) * 6);
  }
  /* «Спокійні» / «Вимкнені» / вимірювання / reduced-motion: світло стоїть. */
  :global(:root[data-effects="calm"]) .frame :is(.spin, .blob),
  :global(:root[data-effects="off"]) .frame :is(.spin, .blob),
  :global(:root.measuring) .frame :is(.spin, .blob) {
    animation: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .spin,
    .blob {
      animation: none;
    }
  }
</style>
