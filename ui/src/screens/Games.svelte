<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { GameProfile } from "../lib/bindings/GameProfile";
  import type { ProfilePreview } from "../lib/bindings/ProfilePreview";
  import { t, tk } from "../lib/i18n";
  import { gm, logLocal, ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import Modal from "../components/Modal.svelte";
  import Page from "../components/Page.svelte";
  import ProcessPicker from "../components/ProcessPicker.svelte";
  import Segmented from "../components/Segmented.svelte";
  import StateView from "../components/StateView.svelte";
  import Toggle from "../components/Toggle.svelte";
  import UpscaleOptions from "../components/UpscaleOptions.svelte";

  const SERVICES = ["WSearch", "SysMain"];

  let profiles = $state<GameProfile[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let editing = $state<GameProfile | null>(null);
  let bgInput = $state("");
  let preview = $state<ProfilePreview | null>(null);
  let busy = $state(false);
  let removing = $state<string | null>(null);

  async function load() {
    loading = true;
    try {
      profiles = await api.profiles();
      gm.watching = await api.gamemodeRunning();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      loading = false;
    }
  }
  onMount(load);

  function newProfile() {
    editing = {
      exe: "",
      game_priority: "above_normal",
      lower_background: [],
      stop_services: [],
      purge_standby_when_low: false,
      upscale: null,
    };
  }

  /** Власні налаштування апскейлу для гри: старт — копія загальних. */
  function toggleOwnUpscale(on: boolean) {
    if (!editing) return;
    editing.upscale = on ? structuredClone($state.snapshot(ui.settings.upscale)) : null;
  }

  function addBackground() {
    const name = bgInput.trim();
    if (editing && name && !editing.lower_background.includes(name)) {
      editing.lower_background = [...editing.lower_background, name];
    }
    bgInput = "";
  }

  async function review() {
    if (!editing) return;
    error = null;
    try {
      preview = await api.profilePreview(editing);
    } catch (e) {
      error = asCmdError(e).message;
    }
  }

  async function save() {
    if (!preview) return;
    busy = true;
    try {
      await api.profileSave(preview.profile);
      logLocal("info", tk("ui.games.saved_log", { exe: preview.profile.exe }));
      preview = null;
      editing = null;
      await restartWatcher();
      await load();
    } catch (e) {
      error = asCmdError(e).message;
    } finally {
      busy = false;
    }
  }

  async function remove(exe: string) {
    await api.profileRemove(exe);
    removing = null;
    await restartWatcher();
    await load();
  }

  /** Стежник читає профілі при старті — після змін перезапускаємо його. */
  async function restartWatcher() {
    if (!gm.watching) return;
    await api.gamemodeStop();
    if ((await api.profiles()).length) await api.gamemodeStart();
  }

  async function toggleWatch(on: boolean) {
    error = null;
    try {
      if (on) await api.gamemodeStart();
      else await api.gamemodeStop();
      gm.watching = on;
    } catch (e) {
      error = asCmdError(e).message;
      gm.watching = false;
    }
  }

  const priorityLabel = (p: GameProfile["game_priority"]) =>
    tk(p === "high" ? "priority.high" : p === "above_normal" ? "priority.above_normal" : "ui.games.priority_unchanged");
</script>

<Page title={tk("ui.games.title")} subtitle={tk("ui.games.subtitle")}>
  <Card>
    <div class="watch">
      <div>
        <h2>{tk("ui.games.watch_title")}</h2>
        <p class="small muted">
          {#if gm.active}
            {tk("ui.games.active", { game: gm.active.game, pid: gm.active.pid })}
          {:else if gm.watching}
            {tk("ui.games.watching", { games: gm.games.join(", ") || profiles.map((p) => p.exe).join(", ") })}
          {:else}
            {tk("ui.games.watch_off")}
          {/if}
        </p>
      </div>
      <Toggle label={tk("ui.games.watch_title")} checked={gm.watching} disabled={!profiles.length} onchange={toggleWatch} />
    </div>
    {#if gm.active}
      <ul class="applied">
        {#each gm.active.applied as a, i (i)}<li class="ok">✓ {t(a)}</li>{/each}
        {#each gm.active.skipped as s, i (i)}<li class="muted">· {t(s)}</li>{/each}
      </ul>
    {:else if gm.lastEnded}
      <p class="small muted ended">
        {tk("ui.games.ended", { game: gm.lastEnded.game, n: gm.lastEnded.result.rolled_back.length })}
        {#each gm.lastEnded.result.failed as f (f.id)}<br /><span class="err">#{f.id}: {f.error}</span>{/each}
      </p>
    {/if}
  </Card>

  {#if error}<p class="err" role="alert">{error}</p>{/if}

  {#if loading}
    <Card><StateView kind="loading" title={tk("ui.common.loading")} /></Card>
  {:else if !profiles.length && !editing}
    <Card>
      <StateView kind="empty" title={tk("ui.games.empty_title")} text={tk("ui.games.empty_text")}>
        {#snippet action()}<Button variant="primary" icon="plus" onclick={newProfile}>{tk("ui.games.add")}</Button>{/snippet}
      </StateView>
    </Card>
  {:else}
    <div class="profiles">
      {#each profiles as p (p.exe)}
        <Card title={p.exe}>
          {#snippet actions()}
            <Button small variant="ghost" onclick={() => (editing = structuredClone($state.snapshot(p)))}>{tk("ui.common.edit")}</Button>
            <Button small variant="ghost" icon="trash" label={tk("ui.common.delete")} onclick={() => (removing = p.exe)} />
          {/snippet}
          <dl class="props">
            <dt>{tk("ui.games.priority")}</dt>
            <dd>{priorityLabel(p.game_priority)}</dd>
            <dt>{tk("ui.games.background")}</dt>
            <dd>{p.lower_background.join(", ") || "—"}</dd>
            <dt>{tk("ui.games.services")}</dt>
            <dd>{p.stop_services.join(", ") || "—"}</dd>
            <dt>{tk("ui.games.purge")}</dt>
            <dd>{p.purge_standby_when_low ? tk("ui.common.yes") : tk("ui.common.no")}</dd>
            <dt>{tk("ui.games.upscale")}</dt>
            <dd>{p.upscale ? tk(`upscale.filter.${p.upscale.filter}`) : tk("ui.games.upscale_global")}</dd>
          </dl>
        </Card>
      {/each}
    </div>
  {/if}

  {#if editing}
    <Card title={editing.exe ? tk("ui.games.edit_title", { exe: editing.exe }) : tk("ui.games.new_title")}>
      <div class="form">
        <ProcessPicker bind:value={editing.exe} label={tk("ui.diag.game")} />
        <div class="field">
          <span class="small muted">{tk("ui.games.priority")}</span>
          <Segmented
            label={tk("ui.games.priority")}
            bind:value={editing.game_priority}
            options={[
              { value: "above_normal", label: tk("priority.above_normal") },
              { value: "high", label: tk("priority.high") },
              { value: "unchanged", label: tk("ui.games.priority_unchanged") },
            ]}
          />
          {#if editing.game_priority === "high"}<p class="small warn">{t({ key: "gm.warn.high_priority", args: {} })}</p>{/if}
        </div>
        <div class="field">
          <label class="small muted" for="bg-input">{tk("ui.games.background")}</label>
          <div class="bg-row">
            <input id="bg-input" bind:value={bgInput} placeholder="chrome.exe" onkeydown={(e) => e.key === "Enter" && addBackground()} />
            <Button small icon="plus" onclick={addBackground}>{tk("ui.common.add")}</Button>
          </div>
          <div class="chips">
            {#each editing.lower_background as b (b)}
              <button
                type="button"
                class="chip"
                aria-label={tk("ui.games.remove_bg", { name: b })}
                onclick={() => editing && (editing.lower_background = editing.lower_background.filter((x) => x !== b))}
                >{b} ✕</button
              >
            {/each}
          </div>
        </div>
        <div class="field">
          <span class="small muted">{tk("ui.games.services")}</span>
          {#each SERVICES as s (s)}
            <div class="toggle-row">
              <span>{s} <span class="small dim">— {tk(`ui.games.service.${s}`)}</span></span>
              <Toggle
                label={s}
                checked={editing.stop_services.includes(s)}
                onchange={(on) =>
                  editing &&
                  (editing.stop_services = on
                    ? [...editing.stop_services, s]
                    : editing.stop_services.filter((x) => x !== s))}
              />
            </div>
          {/each}
        </div>
        <div class="toggle-row">
          <span>{tk("ui.games.purge")}<br /><span class="small dim">{t({ key: "tweak.standby_purge.performance", args: {} })}</span></span>
          <Toggle label={tk("ui.games.purge")} bind:checked={editing.purge_standby_when_low} />
        </div>
        <div class="field">
          <div class="toggle-row">
            <span>{tk("ui.games.upscale_own")}<br /><span class="small dim">{tk("ui.games.upscale_own_hint")}</span></span>
            <Toggle label={tk("ui.games.upscale_own")} checked={editing.upscale !== null} onchange={toggleOwnUpscale} />
          </div>
          {#if editing.upscale}
            <div class="upscale"><UpscaleOptions bind:settings={editing.upscale} /></div>
          {/if}
        </div>
        <div class="form-actions">
          <Button variant="ghost" onclick={() => (editing = null)}>{tk("ui.common.cancel")}</Button>
          <Button variant="primary" disabled={!editing.exe.trim()} onclick={review}>{tk("ui.games.review")}</Button>
        </div>
      </div>
    </Card>
  {/if}

  {#snippet primary()}
    {#if !editing}<Button variant="primary" icon="plus" onclick={newProfile}>{tk("ui.games.add")}</Button>{/if}
  {/snippet}
</Page>

{#if preview}
  <Modal title={tk("ui.games.confirm_title", { exe: preview.profile.exe })} confirmLabel={tk("ui.common.save")} {busy} onconfirm={save} oncancel={() => (preview = null)}>
    <p>{tk("ui.games.confirm_text")}</p>
    <dl class="props">
      <dt>{tk("ui.games.priority")}</dt>
      <dd>{priorityLabel(preview.profile.game_priority)}</dd>
      <dt>{tk("ui.games.background")}</dt>
      <dd class="mono">{preview.profile.lower_background.join(", ") || "—"}</dd>
      <dt>{tk("ui.games.services")}</dt>
      <dd class="mono">{preview.profile.stop_services.join(", ") || "—"}</dd>
      <dt>{tk("ui.games.purge")}</dt>
      <dd>{preview.profile.purge_standby_when_low ? tk("ui.common.yes") : tk("ui.common.no")}</dd>
      <dt>{tk("ui.games.upscale")}</dt>
      <dd>{preview.profile.upscale ? tk(`upscale.filter.${preview.profile.upscale.filter}`) : tk("ui.games.upscale_global")}</dd>
    </dl>
    {#each preview.warnings as w, i (i)}<p class="small warn">⚠ {t(w)}</p>{/each}
  </Modal>
{/if}

{#if removing}
  <Modal title={tk("ui.games.remove_title", { exe: removing })} confirmLabel={tk("ui.common.delete")} danger onconfirm={() => removing && remove(removing)} oncancel={() => (removing = null)}>
    <p>{tk("ui.games.remove_text")}</p>
  </Modal>
{/if}

{#if gm.needsConfirmation}
  <Modal
    title={tk("ui.restore.title")}
    confirmLabel={tk("ui.restore.continue")}
    danger
    onconfirm={async () => {
      await api.gamemodeAllowWithoutRestorePoint();
      gm.needsConfirmation = null;
    }}
    oncancel={() => (gm.needsConfirmation = null)}
  >
    <p>{tk("ui.restore.text", { reason: gm.needsConfirmation.reason })}</p>
    <p class="small">{tk("ui.games.restore_scope", { game: gm.needsConfirmation.game })}</p>
  </Modal>
{/if}

<style>
  .upscale {
    margin-top: calc(var(--space) * 3);
    padding: calc(var(--space) * 4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }
  .watch {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: calc(var(--space) * 4);
  }
  .applied {
    list-style: none;
    margin: calc(var(--space) * 3) 0 0;
    padding: 0;
    display: grid;
    gap: 2px;
    font-size: 13px;
  }
  .ok {
    color: var(--ok);
  }
  .ended {
    margin-top: calc(var(--space) * 3);
  }
  .err {
    color: var(--danger);
  }
  .warn {
    color: var(--warn);
  }
  .profiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: calc(var(--space) * 4);
  }
  .props {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: calc(var(--space) * 1.5) calc(var(--space) * 4);
    margin: 0;
    font-size: 13px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
  }
  .form {
    display: grid;
    gap: calc(var(--space) * 5);
  }
  .field {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .bg-row {
    display: flex;
    gap: calc(var(--space) * 2);
  }
  .bg-row input {
    flex: 1;
    height: 30px;
    padding: 0 calc(var(--space) * 3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    font-family: var(--font-mono);
    font-size: 13px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 1.5);
  }
  .chip {
    padding: 3px 10px;
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    font-size: 12px;
    cursor: pointer;
  }
  .chip:hover {
    border-color: var(--danger);
  }
  .toggle-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: calc(var(--space) * 4);
  }
  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--space) * 2);
  }
</style>
