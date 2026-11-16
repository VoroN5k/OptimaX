<script lang="ts">
  // Верхня панель (docs/DESIGN.md розд. 5): логотип, 5 вкладок із рухомим індикатором, журнал дій
  // і пілюля прав. Пошуку Ctrl+K немає (його немає в програмі — рішення 2026-11-16).
  import { tk } from "../lib/i18n";
  import { openTab, TABS, tabOf } from "../lib/nav";
  import { log, ui } from "../lib/state.svelte";

  /** Ширина колонки вкладки, px (індикатор зсувається на індекс × ширину). */
  const TAB_W = 116;

  const active = $derived(tabOf(ui.screen));
  const index = $derived(Math.max(0, TABS.findIndex((t) => t.id === active)));
  // Права: адміністратор, через службу або без прав і без служби.
  const rights = $derived(
    ui.info?.elevated ? tk("ui.status.admin") : ui.info?.service ? tk("ui.status.service") : tk("ui.status.no_admin"),
  );
  const rightsOk = $derived(!!(ui.info?.elevated || ui.info?.service));
</script>

<header class="top">
  <div class="logo">
    <span class="mark" data-splash-mark aria-hidden="true"></span>
    <span>OptimaX</span>
  </div>

  <nav aria-label={tk("ui.nav.label")} style:--tab-w="{TAB_W}px">
    <span class="thumb" aria-hidden="true" style:transform="translateX({index * TAB_W}px)"></span>
    {#each TABS as t (t.id)}
      <button
        type="button"
        class="tab"
        class:current={t.id === active}
        aria-current={t.id === active ? "page" : undefined}
        onclick={() => openTab(t.id)}>{tk(`ui.tab.${t.id}`)}</button
      >
    {/each}
  </nav>

  <div class="tools">
    <button
      type="button"
      class="ghost"
      aria-expanded={ui.logOpen}
      aria-label={tk("ui.log.button_label", { n: log.lines.length })}
      onclick={() => (ui.logOpen = !ui.logOpen)}
    >
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
        ><path d="M2.5 8a5.5 5.5 0 1 0 1.8-4.1"></path><polyline points="2.5,2 2.5,5 5.5,5"></polyline><polyline points="8,5 8,8 10,9.5"></polyline></svg
      >
      <span class="label">{tk("ui.log.title")}</span>
      <span class="count mono">{log.lines.length}</span>
    </button>
    <span class="pill" title={`${rights} · v${ui.info?.version ?? "…"}`}>
      <span class="dot" class:ok={rightsOk} aria-hidden="true"></span><span class="label">{rights}</span>
    </span>
  </div>
</header>

<style>
  .top {
    position: relative;
    z-index: 3;
    flex: 0 0 auto;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: calc(var(--space) * 3) calc(var(--space) * 6);
    min-height: var(--topbar-h);
    padding: 0 calc(var(--space) * 8);
    border-bottom: 1px solid var(--line-soft);
  }
  .logo {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2.5);
    font-size: var(--fs-body);
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .mark {
    width: 18px;
    height: 18px;
    border-radius: var(--radius-xs);
    background: var(--accent-gradient);
    box-shadow: 0 0 14px rgba(var(--glow), 0.7);
  }
  nav {
    position: relative;
    display: grid;
    grid-template-columns: repeat(5, var(--tab-w));
  }
  .tab {
    height: var(--topbar-h);
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-body);
    font-weight: 500;
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease),
      color var(--dur-fast) var(--ease),
      transform var(--dur-press) var(--ease);
  }
  .tab:hover {
    background: var(--hover);
    color: var(--text);
  }
  .tab:active {
    transform: scale(0.98);
  }
  .tab.current {
    color: var(--text);
  }
  .thumb {
    position: absolute;
    left: 14px;
    bottom: -1px;
    width: 88px;
    height: 2px;
    border-radius: 2px;
    background: var(--text-gradient);
    box-shadow: 0 0 14px rgba(var(--glow), 0.9);
    transition: transform 320ms var(--ease);
  }
  .tools {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .ghost {
    height: var(--ctl-h);
    display: inline-flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    padding: 0 calc(var(--space) * 3);
    border: 1px solid var(--line-ctl);
    border-radius: var(--radius-ctl);
    background: var(--fill-2);
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease),
      border-color var(--dur-fast) var(--ease),
      transform var(--dur-press) var(--ease);
  }
  .ghost:hover {
    background: var(--fill-3);
    border-color: var(--line-strong);
  }
  .ghost:active {
    transform: scale(0.98);
  }
  .ghost[aria-expanded="true"] {
    border-color: rgba(var(--glow), 0.45);
  }
  .count {
    color: var(--text-2);
  }
  .pill {
    height: var(--ctl-h);
    display: inline-flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    padding: 0 calc(var(--space) * 3.5) 0 calc(var(--space) * 2.5);
    border: 1px solid var(--line);
    border-radius: var(--radius-pill);
    font-size: var(--fs-small);
    color: var(--text-2);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--warn);
  }
  .dot.ok {
    background: var(--ok);
  }
  /* Вузьке вікно: журнал — іконка й число, права — крапка (текст — у підказці й для читачів). */
  @media (max-width: 1180px) {
    .label {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0, 0, 0, 0);
      white-space: nowrap;
    }
    .pill {
      padding: 0 calc(var(--space) * 3);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .thumb {
      transition: none;
    }
  }
</style>
