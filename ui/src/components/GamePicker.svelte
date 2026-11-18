<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { GameEntry } from "../lib/bindings/GameEntry";
  import type { LibraryReport } from "../lib/bindings/LibraryReport";
  import { tk } from "../lib/i18n";
  import { logLocal } from "../lib/state.svelte";
  import Button from "./Button.svelte";
  import ProcessPicker from "./ProcessPicker.svelte";
  import Segmented from "./Segmented.svelte";
  import StateView from "./StateView.svelte";

  // Вибір гри: «Мої ігри» (бібліотека Steam / Epic / GOG / Battle.net) або «Запущені процеси».
  // `value` — ім'я exe, як у ProcessPicker; `game` — вибрана гра з бібліотеки (exe може бути ще
  // невідомий — тоді його визначить бекенд під час першого запуску гри).
  let {
    value = $bindable(""),
    game = $bindable(null),
    label,
  }: { value?: string; game?: GameEntry | null; label: string } = $props();

  type Mode = "games" | "processes";
  let mode = $state<Mode>("games");
  let report = $state<LibraryReport | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let query = $state("");
  let showHidden = $state(false);
  let adding = $state(false);
  let addName = $state("");
  let addExe = $state("");
  /** Обкладинки за id гри: рядок — `data:`-адреса, `null` — немає. */
  let covers = $state<Record<string, string | null>>({});

  const games = $derived(report?.games ?? []);
  const hiddenCount = $derived(games.filter((g) => g.hidden).length);
  const visible = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return games.filter((g) => (showHidden || !g.hidden) && (!q || g.name.toLowerCase().includes(q)));
  });
  const problems = $derived(
    (report?.problems ?? []).map((p) => `${tk(`ui.lib.launcher.${p.launcher}`)} (${p.what})`).join(", "),
  );

  async function load(refresh: boolean) {
    loading = true;
    error = null;
    try {
      report = (await api.libraryScan(refresh)) ?? { games: [], problems: [] };
      // Вибраний раніше процес, якого немає в бібліотеці, — показуємо вкладку процесів.
      const known = !value.trim() || games.some((g) => g.exe?.toLowerCase() === value.trim().toLowerCase());
      if (!games.length || !known) mode = "processes";
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }

  onMount(() => void load(false));

  function select(g: GameEntry) {
    game = g;
    value = g.exe ?? "";
  }

  function selectedId(): string | null {
    if (game) return game.id;
    const v = value.trim().toLowerCase();
    return (v && games.find((g) => g.exe?.toLowerCase() === v)?.id) || null;
  }

  // Ручний вибір процесу скасовує вибір гри з бібліотеки.
  $effect(() => {
    if (game && (game.exe ?? "") !== value) game = null;
  });

  async function toggleHidden(g: GameEntry) {
    try {
      await api.librarySetHidden(g.id, !g.hidden);
      g.hidden = !g.hidden;
      if (g.hidden && game?.id === g.id) {
        game = null;
        value = "";
      }
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  async function forget(g: GameEntry) {
    try {
      await api.libraryForgetExe(g.id);
      g.exe = null;
      if (game?.id === g.id) select(g);
      logLocal("info", tk("ui.lib.forgot_log", { game: g.name }));
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  async function addManual() {
    try {
      const g = await api.libraryAddManual(addName.trim(), addExe.trim());
      adding = false;
      addName = "";
      addExe = "";
      await load(false);
      select(g);
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  /** Обкладинка вантажиться, лише коли картка з'являється на екрані. */
  function lazyCover(node: HTMLElement, g: GameEntry) {
    if (!g.has_cover || g.id in covers) return;
    const io = new IntersectionObserver((entries) => {
      if (!entries.some((e) => e.isIntersecting)) return;
      io.disconnect();
      covers[g.id] = null;
      void api
        .libraryCover(g.id)
        .then((c) => (covers[g.id] = c))
        .catch(() => undefined);
    });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  const initial = (name: string) => [...name.trim()][0]?.toUpperCase() ?? "?";
</script>

<div class="gp">
  <div class="head">
    <span class="small muted">{label}</span>
    <Segmented
      label={tk("ui.lib.mode")}
      bind:value={mode}
      options={[
        { value: "games", label: tk("ui.lib.tab_games") },
        { value: "processes", label: tk("ui.lib.tab_processes") },
      ]}
    />
  </div>

  {#if mode === "processes"}
    <ProcessPicker bind:value label={tk("ui.lib.process_label")} />
  {:else if loading && !report}
    <StateView kind="loading" title={tk("ui.common.loading")} />
  {:else if error && !report}
    <StateView kind="error" title={tk("ui.common.error")} text={error} />
  {:else}
    <div class="tools">
      <input
        class="search"
        type="search"
        bind:value={query}
        placeholder={tk("ui.lib.search_placeholder")}
        aria-label={tk("ui.lib.search")}
        autocomplete="off"
        spellcheck="false"
      />
      <Button small icon="refresh" label={tk("ui.lib.refresh")} loading={loading} onclick={() => load(true)} />
      <Button small variant="secondary" icon="plus" onclick={() => (adding = !adding)}>{tk("ui.lib.add")}</Button>
    </div>

    {#if adding}
      <form class="add" onsubmit={(e) => (e.preventDefault(), void addManual())}>
        <input bind:value={addName} placeholder={tk("ui.lib.add_name")} aria-label={tk("ui.lib.add_name")} />
        <input
          class="mono"
          bind:value={addExe}
          placeholder={tk("ui.picker.placeholder")}
          aria-label={tk("ui.lib.add_exe")}
          spellcheck="false"
        />
        <Button small variant="primary" disabled={!addExe.trim()} onclick={() => void addManual()}>{tk("ui.common.add")}</Button>
        <Button small variant="secondary" onclick={() => (adding = false)}>{tk("ui.common.cancel")}</Button>
      </form>
    {/if}

    {#if error}<p class="small err" role="alert">{error}</p>{/if}

    {#if !games.length}
      <StateView kind="empty" title={tk("ui.lib.empty_title")} text={tk("ui.lib.empty_text")} />
    {:else if !visible.length}
      <p class="small muted">{tk("ui.lib.nothing_found", { q: query.trim() })}</p>
    {:else}
      <ul class="grid" aria-label={tk("ui.lib.tab_games")}>
        {#each visible as g (g.id)}
          {@const selected = selectedId() === g.id}
          <li class="card" class:selected class:hidden={g.hidden} use:lazyCover={g}>
            <button type="button" class="pick" aria-pressed={selected} onclick={() => select(g)}>
              <span class="cover" aria-hidden="true">
                {#if covers[g.id]}<img src={covers[g.id]} alt="" />{:else}<span class="letter">{initial(g.name)}</span>{/if}
              </span>
              <span class="name">{g.name}</span>
              <span class="meta">
                <span class="badge">{tk(`ui.lib.launcher.${g.launcher}`)}</span>
                {#if g.hidden}<span class="badge">{tk("ui.lib.hidden_badge")}</span>{/if}
              </span>
              <span class="exe small" class:dim={!g.exe}>
                {g.exe ? tk("ui.lib.exe", { exe: g.exe }) : tk("ui.lib.exe_unknown")}
              </span>
            </button>
            <div class="actions">
              <button type="button" class="link small" onclick={() => toggleHidden(g)}>
                {g.hidden ? tk("ui.lib.unhide") : tk("ui.lib.hide")}
              </button>
              {#if g.exe && g.launcher !== "manual"}
                <button type="button" class="link small" onclick={() => forget(g)}>{tk("ui.lib.forget")}</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}

    {#if hiddenCount}
      <button type="button" class="link small" onclick={() => (showHidden = !showHidden)}>
        {showHidden ? tk("ui.lib.hide_hidden") : tk("ui.lib.show_hidden", { n: hiddenCount })}
      </button>
    {/if}
    {#if problems}<p class="small dim">{tk("ui.lib.problems", { list: problems })}</p>{/if}
  {/if}
</div>

<style>
  .gp {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .head,
  .tools,
  .add {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .head {
    justify-content: space-between;
  }
  input {
    height: var(--ctl-h);
    padding: 0 calc(var(--space) * 3);
    border-radius: var(--radius-ctl);
    border: 1px solid var(--line-ctl);
    background: var(--bg-input);
    color: var(--text);
    font-size: var(--fs-small);
  }
  input:focus-visible {
    outline: none;
    border-color: var(--a1);
  }
  .search {
    flex: 1;
    min-width: 160px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: calc(var(--space) * 3);
    max-height: 420px;
    overflow-y: auto;
  }
  .card {
    display: grid;
    gap: var(--space);
    border-radius: var(--radius-md);
    border: 1px solid var(--line);
    background: var(--tile-solid);
    padding: calc(var(--space) * 2);
    transition: border-color var(--dur-press) var(--ease);
  }
  .card:hover {
    border-color: var(--line-strong);
  }
  .card.selected {
    border-color: var(--a1);
    box-shadow: 0 0 0 1px var(--a1);
  }
  .card.hidden {
    opacity: 0.6;
  }
  .pick {
    display: grid;
    gap: var(--space);
    text-align: left;
    background: none;
    border: 0;
    padding: 0;
    color: inherit;
    cursor: pointer;
  }
  .pick:active {
    transform: scale(0.98);
  }
  .pick:focus-visible {
    outline: 2px solid var(--a1);
    outline-offset: 2px;
    border-radius: var(--radius-sm);
  }
  .cover {
    aspect-ratio: 2 / 3;
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: var(--fill-2);
    display: grid;
    place-items: center;
  }
  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .letter {
    font-size: var(--fs-kpi);
    font-weight: 300;
    color: var(--text-3);
  }
  .name {
    font-size: var(--fs-small);
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space);
  }
  .badge {
    font-size: var(--fs-micro);
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--line);
    color: var(--text-2);
  }
  .exe {
    font-family: var(--font-mono);
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .exe.dim {
    font-family: var(--font-ui);
    color: var(--text-3);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 2);
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--link);
    cursor: pointer;
    justify-self: start;
  }
  .link:focus-visible {
    outline: 2px solid var(--a1);
    outline-offset: 2px;
  }
  .err {
    color: var(--bad);
  }
</style>
