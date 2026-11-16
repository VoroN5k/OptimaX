<script lang="ts">
  import { onMount } from "svelte";
  import { BackdropScene, hexToTriplet, type BackdropColors } from "../lib/backdrop";

  // Фоновий шар «Шовк» (docs/DESIGN.md розд. 9): зерно й canvas з поверхнею. Суто декоративний,
  // без підписів і цифр. `active = false` — нічого не малюється (ефекти не «Повні», вимірювання,
  // фон вимкнено). Згорнуте вікно, canvas поза екраном — пауза; зменшений рух — один статичний кадр.
  let {
    active,
    paused = false,
    grain = true,
    onstats,
  }: {
    active: boolean;
    /** Вікно неактивне: кадр лишається, цикл стоїть. */
    paused?: boolean;
    grain?: boolean;
    onstats?: (s: { fps: number; ms: number }) => void;
  } = $props();

  /** Рух дуже повільний — 20 кадрів/с досить для плавності й удвічі дешевше за 30+. */
  const FRAME_MS = 50;
  /** Роздільність шару не вища за CSS-пікселі: тонкі лінії від масштабування лише м'якшають. */
  const SCALE = 1;

  let anim = $state<HTMLCanvasElement | undefined>();
  let hidden = $state(document.hidden);
  let onScreen = $state(true);
  let reduced = $state(window.matchMedia("(prefers-reduced-motion: reduce)").matches);

  let scene: BackdropScene | null = null;
  let raf = 0;
  let last = 0;
  let w = 0;
  let h = 0;
  let frames = 0;
  let spent = 0;

  function colors(): BackdropColors {
    const css = getComputedStyle(document.documentElement);
    return {
      near: css.getPropertyValue("--glow").replace(/\s/g, "") || "233,205,140",
      far: hexToTriplet(css.getPropertyValue("--thread-idle"), "143,148,170"),
    };
  }

  function resize(): void {
    if (!anim || !scene) return;
    w = window.innerWidth;
    h = window.innerHeight;
    anim.width = Math.round(w * SCALE);
    anim.height = Math.round(h * SCALE);
    anim.getContext("2d")?.setTransform(SCALE, 0, 0, SCALE, 0, 0);
    scene.layout(w, h);
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
      return;
    }
    if (reduced) {
      stop();
      scene.frame(performance.now(), false);
    } else if (!hidden && !paused && onScreen) {
      if (!raf) raf = requestAnimationFrame(loop);
    } else {
      // Пауза (вікно неактивне, простій): цикл стоїть, але нерухомий кадр лишається — інакше фон,
      // змонтований на паузі, був би порожнім.
      stop();
      if (!hidden) scene.frame(performance.now(), true);
    }
  }

  onMount(() => {
    const ctx = anim?.getContext("2d", { alpha: true });
    if (!ctx || !anim) return;
    scene = new BackdropScene(ctx, colors());
    resize();
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onMq = () => (reduced = mq.matches);
    const onVis = () => (hidden = document.hidden);
    const onResize = () => {
      resize();
      if (active && !raf) scene?.frame(performance.now(), !reduced);
    };
    mq.addEventListener("change", onMq);
    document.addEventListener("visibilitychange", onVis);
    window.addEventListener("resize", onResize);
    const io = new IntersectionObserver((e) => (onScreen = e[e.length - 1]?.isIntersecting ?? true));
    io.observe(anim);
    // Палітра змінює кольори акценту — перечитати токени.
    const mo = new MutationObserver(() => {
      scene?.setColors(colors());
      if (active && !raf) scene?.frame(performance.now(), !reduced);
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

  $effect(() => {
    void [active, paused, hidden, onScreen, reduced];
    sync();
  });
</script>

<div class="backdrop" aria-hidden="true" class:off={!active}>
  <canvas bind:this={anim} class="anim"></canvas>
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
  /* Бічні краї поверхні згасають; верх згасає сам (дальні рядки прозорі). */
  .anim {
    -webkit-mask-image: linear-gradient(90deg, transparent, #000 18%, #000 82%, transparent);
    mask-image: linear-gradient(90deg, transparent, #000 18%, #000 82%, transparent);
  }
  .grain {
    opacity: 0.07;
    mix-blend-mode: overlay;
  }
</style>
