<script lang="ts">
  import type { Impact } from "../lib/bindings/Impact";
  import { tk } from "../lib/i18n";

  // Колір завжди дублюється текстом і крапкою-індикатором (не лише колір).
  let { impact }: { impact: Impact } = $props();
  const key = $derived(
    impact === "high" ? "ui.badge.high" : impact === "moderate" ? "ui.badge.moderate" : "ui.badge.info",
  );
</script>

<span class="badge {impact}"><span class="dot" aria-hidden="true"></span>{tk(key)}</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 10px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    border: 1px solid currentColor;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }
  .high {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  .moderate {
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 10%, transparent);
  }
  .info {
    color: var(--info);
    background: color-mix(in srgb, var(--info) 10%, transparent);
  }
</style>
