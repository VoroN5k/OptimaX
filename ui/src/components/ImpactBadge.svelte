<script lang="ts">
  import type { Impact } from "../lib/bindings/Impact";
  import { tk } from "../lib/i18n";
  import StatusPill from "./StatusPill.svelte";

  // Вплив знахідки: колір завжди дублюється текстом (не лише колір). «Впливає на звичні речі» —
  // --bad, «Помірний вплив» — --warn, «Безпечно» — --ok (docs/DESIGN.md розд. 7). `pulse` — для
  // активного вердикту (головна знахідка).
  let { impact, pulse = false }: { impact: Impact; pulse?: boolean } = $props();
  const key = $derived(
    impact === "high" ? "ui.badge.high" : impact === "moderate" ? "ui.badge.moderate" : "ui.badge.info",
  );
  const tone = $derived(impact === "high" ? "bad" : impact === "moderate" ? "warn" : "ok");
</script>

<StatusPill {tone} {pulse} small>{tk(key)}</StatusPill>
