<script lang="ts">
  import { onMount } from "svelte";
  import {
    BackdropScene,
    hexToTriplet,
    type BackdropColors,
    type BackdropData,
    type LaneLayout,
  } from "../lib/backdrop";
  import { tk } from "../lib/i18n";

  // Фоновий шар (docs/DESIGN.md розд. 9): зерно, анімований розмитий canvas і різкий canvas
  // підписів (малюється лише при зміні розміру чи даних). Підписи й легенда — тільки для
  // реального запису; декоративний фон без них. `active = false` — нічого не малюється
  // (ефекти не «Повні», вимірювання, фон вимкнено). Згорнуте вікно, canvas поза екраном —
  // пауза; зменшений рух — один статичний кадр.
  let {
    active,
    data,
    game = null,
    grain = true,
    onstats,
  }: {
    active: boolean;
    data: BackdropData | null;
    game?: string | null;
    grain?: boolean;
    onstats?: (s: { fps: number; ms: number }) => void;
  } = $props();

  const FRAME_MS = 33;
  const DPR_MAX = 2;

  let anim = $state<HTMLCanvasElement | undefined>();
  let labels = $state<HTMLCanvasElement | undefined>();
  let hidden = $state(document.hidden);
  let onScreen = $state(true);
  let reduced = $state(window.matchMedia("(prefers-reduced-motion: reduce)").matches);

  let scene: BackdropScene | null = null;
  let lanes: LaneLayout | null = null;
  let raf = 0;
  let last = 0;
  let w = 0;
  let h = 0;
  let frames = 0;
  let spent = 0;

  function colors(): BackdropColors {
    const css = getComputedStyle(document.documentElement);
    const v = (name: string, fb: string) => hexToTriplet(css.getPropertyValue(name), fb);
    return {
      run: v("--thread-work", "108,124,255"),
      wait: v("--warn", "255,178,122"),
      idle: v("--thread-idle", "143,148,170"),
      frame: v("--a2", "201,164,94"),
      over: css.getPropertyValue("--warn-glow").replace(/\s/g, "") || "255,138,91",
      cursor: css.getPropertyValue("--glow").replace(/\s/g, "") || "233,205,140",
    };
  }

  function resize(): void {
    if (!anim || !labels || !scene) return;
    w = window.innerWidth;
    h = window.innerHeight;
    const dpr = Math.min(DPR_MAX, window.devicePixelRatio || 1);
    for (const c of [anim, labels]) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
      c.getContext("2d")?.setTransform(dpr, 0, 0, dpr, 0, 0);
    }
    lanes = scene.layout(w, h);
    drawLabels();
  }

  /** Підписи доріжок і легенда — лише для запису. */
  function drawLabels(): void {
    const ctx = labels?.getContext("2d");
    if (!ctx || !lanes) return;
    ctx.clearRect(0, 0, w, h);
    if (!data || !active) return;
    const css = getComputedStyle(document.documentElement);
    const mono = css.getPropertyValue("--font-mono") || "monospace";
    const text2 = css.getPropertyValue("--text-2").trim() || "#B2B0AA";
    const text3 = css.getPropertyValue("--text-3").trim() || "#8F8D88";
    const c = colors();
    ctx.font = `10px ${mono}`;
    ctx.textBaseline = "middle";
    data.lanes.forEach((l, i) => {
      const y = lanes?.y[i];
      if (y === undefined) return;
      ctx.fillStyle = text3;
      ctx.fillText(tk(`ui.bg.lane.${l.key}`), 32, y - 9);
    });
    ctx.fillStyle = text3;
    ctx.fillText(tk("ui.bg.frames"), 32, lanes.stripY - 14);
    // Легенда — під смугою кадрів (над нижньою панеллю дій), щоб не налазити на шапку сторінки.
    let x = 32;
    const ly = lanes.stripY + 40;
    const item = (rgb: string, label: string) => {
      ctx.fillStyle = `rgba(${rgb},0.8)`;
      ctx.fillRect(x, ly - 4, 8, 8);
      ctx.fillStyle = text2;
      ctx.fillText(label, x + 14, ly);
      x += 14 + ctx.measureText(label).width + 18;
    };
    const head = (label: string) => {
      ctx.fillStyle = text3;
      ctx.fillText(label, x, ly);
      x += ctx.measureText(label).width + 12;
    };
    head(tk("ui.bg.legend.threads", { game: game ?? "" }));
    item(c.run, tk("ui.bg.legend.run"));
    item(c.wait, tk("ui.bg.legend.wait"));
    item(c.idle, tk("ui.bg.legend.idle"));
    x += 14;
    head(tk("ui.bg.legend.frames"));
    item(c.frame, tk("ui.bg.legend.frame"));
    item(c.over, tk("ui.bg.legend.spike"));
    ctx.fillStyle = text3;
    ctx.fillText(tk("ui.bg.legend.replay"), x + 6, ly);
  }

  const loop = (now: number): void => {
    raf = requestAnimationFrame(loop);
    if (now - last < FRAME_MS) return;
    last = now;
    const s = performance.now();
    scene?.frame(now, true);
    spent += performance.now() - s;
    frames++;
  };

  function stop(): void {
    cancelAnimationFrame(raf);
    raf = 0;
  }

  function sync(): void {
    if (!scene || !anim) return;
    if (!active) {
      stop();
      anim.getContext("2d")?.clearRect(0, 0, w, h);
      drawLabels();
      return;
    }
    if (reduced) {
      stop();
      scene.frame(performance.now(), false);
    } else if (!hidden && onScreen) {
      if (!raf) raf = requestAnimationFrame(loop);
    } else {
      stop();
    }
  }

  onMount(() => {
    const ctx = anim?.getContext("2d", { alpha: true });
    if (!ctx || !anim) return;
    scene = new BackdropScene(ctx, colors());
    scene.setData(data);
    resize();
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onMq = () => (reduced = mq.matches);
    const onVis = () => (hidden = document.hidden);
    const onResize = () => {
      resize();
      if (reduced && active) scene?.frame(performance.now(), false);
    };
    mq.addEventListener("change", onMq);
    document.addEventListener("visibilitychange", onVis);
    window.addEventListener("resize", onResize);
    const io = new IntersectionObserver((e) => (onScreen = e[e.length - 1]?.isIntersecting ?? true));
    io.observe(anim);
    // Палітра змінює кольори акценту — перечитати токени.
    const mo = new MutationObserver(() => {
      scene?.setColors(colors());
      drawLabels();
    });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-palette"] });
    const statsTimer = onstats
      ? setInterval(() => {
          if (frames > 0) onstats({ fps: frames, ms: spent / frames });
          frames = 0;
          spent = 0;
        }, 1000)
      : undefined;
    sync();
    return () => {
      stop();
      clearInterval(statsTimer);
      io.disconnect();
      mo.disconnect();
      mq.removeEventListener("change", onMq);
      document.removeEventListener("visibilitychange", onVis);
      window.removeEventListener("resize", onResize);
    };
  });

  // Нові дані запису — новий відлік і підписи.
  $effect(() => {
    void data;
    void game;
    if (!scene) return;
    scene.setData(data);
    resize();
    sync();
  });

  $effect(() => {
    void [active, hidden, onScreen, reduced];
    sync();
  });
</script>

<div class="backdrop" aria-hidden="true" class:off={!active}>
  <canvas bind:this={anim} class="anim"></canvas>
  <canvas bind:this={labels} class="labels"></canvas>
</div>
{#if grain}
  <svg class="grain" aria-hidden="true" width="100%" height="100%">
    <filter id="bd-noise">
      <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" stitchTiles="stitch" />
      <feColorMatrix type="saturate" values="0" />
    </filter>
    <rect width="100%" height="100%" filter="url(#bd-noise)" />
  </svg>
{/if}

<style>
  .backdrop,
  .grain {
    position: fixed;
    inset: 0;
    z-index: 0;
    pointer-events: none;
  }
  .backdrop.off {
    display: none;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }
  /* Краї анімованого шару згасають, розмиття не дає йому конкурувати з текстом. Підписи — різкі
     й без маски, інакше біля краю їх не прочитати. */
  .anim {
    -webkit-mask-image: radial-gradient(ellipse 90% 70% at 50% 40%, #000 30%, transparent);
    mask-image: radial-gradient(ellipse 90% 70% at 50% 40%, #000 30%, transparent);
    filter: blur(2.5px);
  }
  .grain {
    opacity: 0.07;
    mix-blend-mode: overlay;
  }
</style>
