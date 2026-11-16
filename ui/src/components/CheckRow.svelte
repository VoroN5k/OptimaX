<script lang="ts">
  import type { Snippet } from "svelte";

  // Рядок «що можна перевірити» (docs/DESIGN.md розд. 7, CheckRow): бейдж-номер 26×26 і текст;
  // між рядками межа, hover — легкий фон.
  let { n, children }: { n: number; children: Snippet } = $props();
</script>

<li class="row">
  <span class="badge mono" aria-hidden="true">{n}</span>
  <span class="text">{@render children()}</span>
</li>

<style>
  .row {
    display: grid;
    grid-template-columns: 36px minmax(0, 1fr);
    gap: calc(var(--space) * 2);
    align-items: center;
    padding: calc(var(--space) * 2.5) calc(var(--space) * 2);
    margin: 0 calc(var(--space) * -2);
    border-top: 1px solid var(--line-soft);
    border-radius: var(--radius-ctl);
    font-size: var(--fs-body);
    line-height: 20px;
    list-style: none;
    transition: background-color var(--dur-fast) var(--ease);
  }
  .row:last-child {
    border-bottom: 1px solid var(--line-soft);
  }
  .row:hover {
    background: var(--hover);
  }
  .badge {
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    background: linear-gradient(135deg, rgba(var(--glow), 0.3), rgba(var(--glow), 0.08));
    color: var(--a1);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: var(--fs-small);
  }
</style>
