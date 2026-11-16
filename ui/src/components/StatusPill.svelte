<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  // Пілюля статусу (docs/DESIGN.md розд. 7, StatusPill): колір і межа — від вердикту, завжди з
  // текстом і крапкою чи іконкою (не лише колір). Пульсуюче кільце — лише для «Помірний вплив» і
  // гірших (`warn`, `bad`), і лише коли `pulse`.
  let {
    tone,
    icon,
    pulse = false,
    small = false,
    title,
    children,
  }: {
    tone: "ok" | "warn" | "bad" | "neutral";
    /** Іконка замість крапки. */
    icon?: string;
    pulse?: boolean;
    small?: boolean;
    title?: string;
    children: Snippet;
  } = $props();

  const pulsing = $derived(pulse && (tone === "warn" || tone === "bad"));
</script>

<span class="pill {tone}" class:small {title}>
  {#if icon}
    <Icon name={icon} size={small ? 12 : 13} />
  {:else}
    <span class="dot" class:pulse={pulsing} aria-hidden="true"></span>
  {/if}
  {@render children()}
</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    padding: var(--space) calc(var(--space) * 3) var(--space) calc(var(--space) * 2.5);
    border-radius: var(--radius-pill);
    border: 1px solid color-mix(in srgb, currentColor 40%, transparent);
    background: color-mix(in srgb, currentColor 8%, transparent);
    font-size: var(--fs-small);
    font-weight: 600;
    white-space: nowrap;
  }
  .small {
    gap: 6px;
    padding: 3px calc(var(--space) * 2.5) 3px calc(var(--space) * 2);
  }
  .ok {
    color: var(--ok);
  }
  .warn {
    color: var(--warn);
  }
  .bad {
    color: var(--bad);
  }
  .neutral {
    color: var(--text-3);
  }
  .dot {
    position: relative;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
    flex-shrink: 0;
  }
  .dot.pulse::after {
    content: "";
    position: absolute;
    inset: -4px;
    border-radius: 50%;
    border: 1px solid currentColor;
    animation: pulse 2.4s ease-out infinite;
  }
  @keyframes pulse {
    0% {
      transform: scale(0.7);
      opacity: 0.7;
    }
    100% {
      transform: scale(1.9);
      opacity: 0;
    }
  }
  /* Нескінченна анімація — лише в режимі «Повні» і не під час вимірювань. */
  :global(:root[data-effects="calm"]) .dot.pulse::after,
  :global(:root[data-effects="off"]) .dot.pulse::after,
  :global(:root.measuring) .dot.pulse::after {
    animation: none;
    opacity: 0;
  }
  @media (prefers-reduced-motion: reduce) {
    .dot.pulse::after {
      animation: none;
      opacity: 0;
    }
  }
</style>
