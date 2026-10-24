<script lang="ts">
  import type { CleanupVerdict } from "../lib/bindings/CleanupVerdict";
  import { tk } from "../lib/i18n";
  import Icon from "./Icon.svelte";

  // Вердикт очищення: завжди іконка + текст, не лише колір (CLAUDE.md, 10.3).
  let { verdict }: { verdict: CleanupVerdict } = $props();
  const icons: Record<CleanupVerdict, string> = {
    safe: "check",
    consequences: "warning",
    protected: "lock",
    unknown: "help",
  };
</script>

<span class="badge {verdict}" title={tk(`ui.clean.verdict_hint.${verdict}`)}>
  <Icon name={icons[verdict]} size={13} />{tk(`ui.clean.verdict.${verdict}`)}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 9px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    border: 1px solid currentColor;
  }
  .safe {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 10%, transparent);
  }
  .consequences {
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 10%, transparent);
  }
  .protected {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  .unknown {
    color: var(--text-muted);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
  }
</style>
