<script lang="ts">
  import { onMount } from "svelte";
  import { api, asCmdError } from "../lib/api";
  import type { PlanHandle } from "../lib/bindings/PlanHandle";
  import type { UiDesired } from "../lib/bindings/UiDesired";
  import { t, tk } from "../lib/i18n";
  import { flashBg, logLocal } from "../lib/state.svelte";
  import ChangePreview from "./ChangePreview.svelte";
  import Modal from "./Modal.svelte";

  // План будує бекенд і зберігає в себе; тут лише показ і підтвердження за номером плану.
  let { desired, onclose }: { desired: UiDesired; onclose: (applied: boolean) => void } = $props();

  let handle = $state<PlanHandle | null>(null);
  let error = $state<string | null>(null);
  let restoreReason = $state<string | null>(null);
  let busy = $state(false);

  onMount(async () => {
    try {
      handle = await api.tweakPlan(desired);
    } catch (e) {
      error = asCmdError(e).message;
    }
  });

  async function apply(allowWithoutRestorePoint: boolean) {
    if (!handle) return;
    busy = true;
    try {
      const r = await api.tweakApply(handle.id, allowWithoutRestorePoint);
      if (r.restore_point?.status === "created") {
        logLocal("info", tk("ui.tweak.restore_created", { n: r.restore_point.sequence }));
      }
      logLocal("info", tk("ui.tweak.applied", { title: t(handle.plan.info.title) }));
      flashBg("ok");
      onclose(true);
    } catch (e) {
      const err = asCmdError(e);
      if (err.kind === "restore_point") {
        restoreReason = err.message;
      } else {
        error = err.message;
        flashBg("warn");
      }
    } finally {
      busy = false;
    }
  }
</script>

{#if restoreReason !== null}
  <Modal
    title={tk("ui.restore.title")}
    confirmLabel={tk("ui.restore.continue")}
    danger
    {busy}
    onconfirm={() => apply(true)}
    oncancel={() => onclose(false)}
  >
    <p>{tk("ui.restore.text", { reason: restoreReason })}</p>
  </Modal>
{:else}
  <Modal
    title={handle ? t(handle.plan.info.title) : tk("ui.tweak.loading")}
    confirmLabel={tk("ui.tweak.apply")}
    {busy}
    onconfirm={() => (handle && handle.plan.changes.length > 0 ? apply(false) : onclose(false))}
    oncancel={() => onclose(false)}
  >
    {#if error}
      <p class="error">{error}</p>
    {:else if handle}
      <p>{t(handle.plan.info.what)}</p>
      <ChangePreview plan={handle.plan} />
      {#if handle.plan.changes.length > 0}
        <p class="small dim">{tk("ui.tweak.unmeasured_hint")}</p>
      {/if}
    {:else}
      <p>{tk("ui.tweak.loading")}</p>
    {/if}
  </Modal>
{/if}

<style>
  .error {
    color: var(--danger);
  }
</style>
