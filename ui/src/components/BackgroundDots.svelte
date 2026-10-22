<script lang="ts">
  import { onMount } from "svelte";
  import { DotField, type DotsMode, type DotsQuality } from "../lib/dots";

  // Обгортка анімації: вирішує, коли малювати. Повністю вимикається, якщо вікно приховане
  // або без фокуса, ззовні передано `enabled = false` (режим гри, запис, економний режим,
  // «Вимкнена» в налаштуваннях). prefers-reduced-motion → статичний кадр.
  let {
    enabled,
    quality,
    mode,
    leftInset,
    onstats,
  }: {
    enabled: boolean;
    quality: DotsQuality;
    mode: DotsMode;
    leftInset: number;
    onstats?: (s: ReturnType<DotField["stats"]>) => void;
  } = $props();

  let canvas = $state<HTMLCanvasElement | undefined>();
  let field: DotField | null = null;
  let visible = $state(!document.hidden);
  let focused = $state(document.hasFocus());
  let reduced = $state(false);

  function sync() {
    if (!field) return;
    field.setQuality(quality);
    field.setMode(mode);
    if (reduced) {
      field.renderStatic();
    } else if (enabled && visible && focused) {
      field.start();
    } else {
      field.stop();
    }
  }

  onMount(() => {
    if (!canvas) return;
    field = new DotField(canvas, leftInset);
    field.setQuality(quality);
    field.resize();
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    reduced = mq.matches;
    const onMq = () => {
      reduced = mq.matches;
      sync();
    };
    const onVis = () => (visible = !document.hidden);
    const onFocus = () => (focused = true);
    const onBlur = () => (focused = false);
    const onResize = () => field?.resize();
    const onMove = (e: PointerEvent) => field?.setMouse(e.clientX, e.clientY);
    mq.addEventListener("change", onMq);
    document.addEventListener("visibilitychange", onVis);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    window.addEventListener("resize", onResize);
    window.addEventListener("pointermove", onMove, { passive: true });
    const statsTimer = onstats ? setInterval(() => field && onstats(field.stats()), 1000) : undefined;
    sync();
    return () => {
      field?.stop();
      clearInterval(statsTimer);
      mq.removeEventListener("change", onMq);
      document.removeEventListener("visibilitychange", onVis);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("pointermove", onMove);
    };
  });

  $effect(() => {
    // Реакція на зміну будь-якої умови.
    void [enabled, quality, mode, visible, focused, reduced];
    sync();
  });
</script>

<canvas bind:this={canvas} class="bg" aria-hidden="true"></canvas>
<div class="vignette" aria-hidden="true"></div>

<style>
  .bg,
  .vignette {
    position: fixed;
    inset: 0;
    z-index: 0;
    pointer-events: none;
  }
  /* Розмір і позицію canvas задає DotField (лише при зміні розміру вікна). */
  .bg {
    inset: auto;
  }
  /* Приглушення до фону на краях — анімація не заважає читати текст. */
  .vignette {
    background:
      radial-gradient(ellipse at calc(50% + var(--sidebar-width) / 2) 50%, transparent 38%, var(--bg-app) 85%),
      linear-gradient(color-mix(in srgb, var(--bg-app) 20%, transparent), color-mix(in srgb, var(--bg-app) 20%, transparent));
  }
</style>
