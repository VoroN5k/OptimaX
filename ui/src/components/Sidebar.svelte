<script lang="ts">
  import { ui, type ScreenId } from "../lib/state.svelte";
  import { tk } from "../lib/i18n";
  import Icon from "./Icon.svelte";

  const groups: { key: string; items: ScreenId[] }[] = [
    { key: "ui.nav.group.main", items: ["analysis", "optimization", "games", "diagnosis"] },
    { key: "ui.nav.group.device", items: ["hardware", "laptop", "network"] },
    { key: "ui.nav.group.tools", items: ["upscale", "cleanup", "apps", "startup", "appearance"] },
    { key: "ui.nav.group.system", items: ["backups", "settings"] },
  ];
  const ready = new Set<ScreenId>([
    "analysis", "optimization", "games", "diagnosis", "hardware", "laptop", "network", "upscale", "cleanup", "backups", "settings",
  ]);
</script>

<nav class="sidebar" aria-label={tk("ui.nav.label")}>
  <div class="logo">
    <span class="mark" aria-hidden="true"></span>
    <div>
      <strong>OptimaX</strong>
      <span class="small dim">by X44TO</span>
    </div>
  </div>

  {#each groups as g (g.key)}
    <div class="group">
      <span class="group-title">{tk(g.key)}</span>
      {#each g.items as id (id)}
        <button
          type="button"
          class="item"
          class:active={ui.screen === id}
          aria-current={ui.screen === id ? "page" : undefined}
          onclick={() => (ui.screen = id)}
        >
          <Icon name={id} />
          <span>{tk(`ui.nav.${id}`)}</span>
          {#if !ready.has(id)}<span class="soon small">{tk("ui.nav.soon")}</span>{/if}
        </button>
      {/each}
    </div>
  {/each}

  <div class="status small">
    <span
      class="dot"
      class:ok={ui.info?.elevated}
      title={ui.info?.elevated ? tk("ui.status.admin") : tk("ui.status.no_admin")}
      aria-hidden="true"
    ></span>
    <span>{ui.info?.elevated ? tk("ui.status.admin") : tk("ui.status.no_admin")}</span>
    <span class="dim ver">v{ui.info?.version ?? "…"}</span>
  </div>
</nav>

<style>
  .sidebar {
    position: relative;
    z-index: 2;
    width: var(--sidebar-width);
    flex-shrink: 0;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 4) calc(var(--space) * 3);
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    overflow-x: hidden;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--border-strong) transparent;
  }
  .logo {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    padding: 0 calc(var(--space) * 2) calc(var(--space) * 2);
  }
  .logo div {
    display: grid;
    line-height: 1.2;
  }
  .logo strong {
    font-size: 16px;
  }
  .mark {
    width: 30px;
    height: 30px;
    border-radius: 9px;
    background:
      radial-gradient(circle, var(--bg-sidebar) 32%, transparent 34%),
      var(--accent-gradient);
    box-shadow: 0 0 18px var(--accent-glow);
  }
  .group {
    display: grid;
    gap: 2px;
  }
  .group-title {
    padding: 0 calc(var(--space) * 3) calc(var(--space) * 1);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .item {
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 3);
    height: 34px;
    min-width: 0;
    padding: 0 calc(var(--space) * 3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    text-align: left;
    transition: background-color var(--dur) var(--ease), color var(--dur) var(--ease);
  }
  .item:hover {
    color: var(--text);
    background: var(--bg-card);
  }
  .item.active {
    color: var(--text);
    background: linear-gradient(90deg, color-mix(in srgb, var(--accent-from) 22%, transparent), transparent);
    box-shadow: inset 2px 0 0 var(--accent-from);
  }
  .item span:first-of-type {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .soon {
    flex-shrink: 0;
    font-size: 10px;
    line-height: 16px;
    padding: 0 5px;
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    color: var(--text-dim);
  }
  .status {
    margin-top: auto;
    display: flex;
    align-items: center;
    gap: calc(var(--space) * 2);
    padding: calc(var(--space) * 3);
    border-top: 1px solid var(--border);
    color: var(--text-muted);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--warn);
    box-shadow: 0 0 8px var(--warn);
  }
  .dot.ok {
    background: var(--ok);
    box-shadow: 0 0 8px var(--ok);
  }
  .ver {
    margin-left: auto;
  }
</style>
