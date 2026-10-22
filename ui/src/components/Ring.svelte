<script lang="ts">
  // Кільцевий індикатор із плавним заповненням (stroke-dashoffset), градієнт акценту.
  let {
    value,
    label,
    caption,
    size = 112,
  }: { value: number; label: string; caption?: string; size?: number } = $props();

  const stroke = 9;
  const r = $derived((size - stroke) / 2);
  const c = $derived(2 * Math.PI * r);
  const clamped = $derived(Math.max(0, Math.min(100, value)));
  const gid = `ring-${Math.random().toString(36).slice(2, 8)}`;
</script>

<div class="ring" style:width="{size}px" role="img" aria-label="{label}: {Math.round(clamped)}%">
  <svg width={size} height={size} viewBox="0 0 {size} {size}" aria-hidden="true">
    <defs>
      <linearGradient id={gid} x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="var(--accent-from)" />
        <stop offset="1" stop-color="var(--accent-to)" />
      </linearGradient>
    </defs>
    <circle cx={size / 2} cy={size / 2} {r} stroke="var(--border)" stroke-width={stroke} fill="none" />
    <circle
      class="arc"
      cx={size / 2}
      cy={size / 2}
      {r}
      stroke="url(#{gid})"
      stroke-width={stroke}
      fill="none"
      stroke-linecap="round"
      stroke-dasharray={c}
      stroke-dashoffset={c * (1 - clamped / 100)}
      transform="rotate(-90 {size / 2} {size / 2})"
    />
  </svg>
  <div class="center">
    <strong>{Math.round(clamped)}%</strong>
    {#if caption}<span class="small muted">{caption}</span>{/if}
  </div>
  <span class="label small muted">{label}</span>
</div>

<style>
  .ring {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .arc {
    transition: stroke-dashoffset var(--dur-slow) var(--ease);
  }
  .center {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: calc(100% - 26px);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
  }
  strong {
    font-size: 22px;
    font-weight: 600;
  }
  .label {
    text-align: center;
  }
</style>
