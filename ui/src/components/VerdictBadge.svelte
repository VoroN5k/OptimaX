<script lang="ts">
  import type { CleanupVerdict } from "../lib/bindings/CleanupVerdict";
  import { tk } from "../lib/i18n";
  import StatusPill from "./StatusPill.svelte";

  // Вердикт очищення: завжди іконка + текст, не лише колір (CLAUDE.md, 10.3).
  let { verdict }: { verdict: CleanupVerdict } = $props();
  const icons: Record<CleanupVerdict, string> = {
    safe: "check",
    consequences: "warning",
    protected: "lock",
    unknown: "help",
  };
  const tones: Record<CleanupVerdict, "ok" | "warn" | "bad" | "neutral"> = {
    safe: "ok",
    consequences: "warn",
    protected: "bad",
    unknown: "neutral",
  };
</script>

<StatusPill tone={tones[verdict]} icon={icons[verdict]} small title={tk(`ui.clean.verdict_hint.${verdict}`)}
  >{tk(`ui.clean.verdict.${verdict}`)}</StatusPill
>
