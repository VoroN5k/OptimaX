// Заставка запуску: хаос із точок і штрихів → знак «OX» у кільці → хвиля-кільце →
// знак зменшується й летить на своє місце в сайдбарі, кільце розходиться в кільця
// фонового кола, контент з'являється (fade + 8px).
//
// CLAUDE.md 10.5 обмежує анімації 120–400 мс; для заставки запуску користувач дозволив виняток.
// Кадри — синхронно з vsync (rAF): таймер + rAF, як у фону, дає нерівний крок кадрів, а на 30 FPS
// рух точок помітно смикається; заставка триває ≈2 с, тож це не постійне навантаження.
// Решта обмежень — як у фону (lib/dots.ts): без алокацій у кадрі (Float32Array, заздалегідь намальовані спрайти), DPR ≤ 1.5.
//
// Запуск не затримується штучно: заставка стартує одночасно із завантаженням даних бекенду.
// Відповів пізніше, ніж зібрався знак, — знак чекає; раніше — анімація догравається (≈2 с).
// Клік або будь-яка клавіша пропускають її.
// Не показується: prefers-reduced-motion, приховане вікно, «Анімація фону: Вимкнена» або
// економний режим (два останні — з localStorage, бо налаштування ще не прочитані).

import { hexToRgb, makeSprite } from "./dots";

const KEY = "optimax.splash";
const DPR_MAX = 1.5;
const BUCKETS = 12; // відтінки градієнта зліва направо
const SPRITE_PX = 48;
const FONT_WAIT_MS = 250; // Inter вбудований локально; довше не чекаємо
const GATHER_FROM_MS = 180; // найраніший старт збирання
const GATHER_SPREAD_MS = 260; // розкид старту між точками
const GATHER_MS = 620;
const GATHERED_MS = GATHER_FROM_MS + GATHER_SPREAD_MS + GATHER_MS;
const HOLD_MS = 360; // пауза з хвилею після збирання
const WAVE_MS = 700;
const OUT_MS = 560;
const SKIP_MS = 200;
const MAX_WAIT_MS = 10_000; // бекенд мовчить — заставку прибрати все одно
const LEFT_INSET = 225; // ширина сайдбара: центр фонового кола — посередині контенту
const BG_RINGS = 16;

const KIND_RING = 0;
const KIND_LETTER = 1;
const KIND_DUST = 2;

/** Керування заставкою з `main.ts`. */
export interface Splash {
  /** Інтерфейс змонтовано: можна летіти до знака в сайдбарі. */
  ready(): void;
  /** Запуск не вдався: швидко прибрати заставку. */
  cancel(): void;
}

/** Запам'ятати, чи дозволена заставка (читається до завантаження налаштувань). */
export function rememberSplash(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? "1" : "0");
  } catch {
    // Без сховища заставка просто показуватиметься за замовчуванням.
  }
}

function allowed(): boolean {
  if (document.hidden) return false;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return false;
  try {
    return localStorage.getItem(KEY) !== "0";
  } catch {
    return true;
  }
}

const easeOut = (u: number) => 1 - (1 - u) * (1 - u);
const easeInOut = (u: number) => (u < 0.5 ? 4 * u * u * u : 1 - Math.pow(-2 * u + 2, 3) / 2);
const clamp01 = (u: number) => (u < 0 ? 0 : u > 1 ? 1 : u);

/** Стартує заставку або повертає `null`, якщо її не треба показувати. */
export function startSplash(): Splash | null {
  if (!allowed()) return null;
  const run = new SplashRun();
  return { ready: () => run.ready(), cancel: () => run.cancel() };
}

class SplashRun {
  private bg = document.createElement("div");
  private canvas = document.createElement("canvas");
  private ctx: CanvasRenderingContext2D | null;
  private w = window.innerWidth;
  private h = window.innerHeight;
  private dpr = Math.min(DPR_MAX, window.devicePixelRatio || 1);
  private logoX = this.w / 2;
  private logoY = this.h / 2;
  private logoR = Math.round(Math.min(this.w, this.h) * 0.15);
  private sprites: HTMLCanvasElement[] = [];
  private colors: string[] = [];
  private waveColor = "";
  // Дані точок (довжина = n): старт, кінець підйому, ціль у знаку, ціль виходу, затримка, розмір.
  private n = 0;
  private kind = new Uint8Array(0);
  private bucket = new Uint8Array(0);
  private streak = new Uint8Array(0);
  private sx = new Float32Array(0);
  private sy = new Float32Array(0);
  private mx = new Float32Array(0);
  private my = new Float32Array(0);
  private tx = new Float32Array(0);
  private ty = new Float32Array(0);
  private ex = new Float32Array(0);
  private ey = new Float32Array(0);
  private delay = new Float32Array(0);
  private size = new Float32Array(0);
  private markScale = 1;
  private t0 = -1;
  private isReady = false;
  private skipped = false;
  private outAt = -1;
  private outMs = OUT_MS;
  private bgOpacity = 1;
  private finished = false;
  private raf = 0;
  private frames = 0;
  private spentMs = 0;

  constructor() {
    const root = document.documentElement;
    root.classList.add("splashing", "splash-mark");
    // Легка сітка з крапок на фоні — статична, лише CSS. Фон перехоплює кліки, щоб вони
    // не влучили в ще невидимий інтерфейс під ним.
    this.bg.setAttribute("aria-hidden", "true");
    this.bg.style.cssText =
      "position:fixed;inset:0;z-index:1000;cursor:default;" +
      "background:radial-gradient(circle, color-mix(in srgb, var(--text-dim) 30%, transparent) 1px, transparent 1.6px) 0 0 / 22px 22px, var(--bg-app);";
    this.canvas.setAttribute("aria-hidden", "true");
    this.canvas.style.cssText = `position:fixed;inset:0;z-index:1001;pointer-events:none;width:${this.w}px;height:${this.h}px;`;
    this.canvas.width = Math.round(this.w * this.dpr);
    this.canvas.height = Math.round(this.h * this.dpr);
    this.ctx = this.canvas.getContext("2d", { alpha: true });
    document.body.append(this.bg, this.canvas);
    window.addEventListener("pointerdown", this.skip, true);
    window.addEventListener("keydown", this.skip, true);
    void this.init();
  }

  ready(): void {
    this.isReady = true;
  }

  cancel(): void {
    this.markScale = 0;
    this.isReady = true;
    this.skipped = true;
    if (this.t0 < 0) this.finish();
  }

  private skip = (): void => {
    this.skipped = true;
  };

  private async init(): Promise<void> {
    const css = getComputedStyle(document.documentElement);
    const from = hexToRgb(css.getPropertyValue("--accent-from") || "#6E7BFF");
    const to = hexToRgb(css.getPropertyValue("--accent-to") || "#8B5CF6");
    for (let i = 0; i < BUCKETS; i++) {
      const k = i / (BUCKETS - 1);
      const rgb: [number, number, number] = [
        Math.round(from[0] + (to[0] - from[0]) * k),
        Math.round(from[1] + (to[1] - from[1]) * k),
        Math.round(from[2] + (to[2] - from[2]) * k),
      ];
      this.sprites.push(makeSprite(rgb, SPRITE_PX));
      this.colors.push(`rgb(${rgb[0]},${rgb[1]},${rgb[2]})`);
    }
    this.waveColor = `rgb(${from[0]},${from[1]},${from[2]})`;
    const font = `600 ${Math.round(this.logoR * 0.95)}px Inter, sans-serif`;
    try {
      await Promise.race([document.fonts.load(font), new Promise((r) => setTimeout(r, FONT_WAIT_MS))]);
    } catch {
      // Шрифт не підвантажився — малюємо системним, форма знака та сама.
    }
    if (this.finished) return;
    this.layout(font);
    this.raf = requestAnimationFrame(this.frame);
  }

  /** Цілі точок: кільце, растр літер «OX», пил навколо. Алокації — лише тут. */
  private layout(font: string): void {
    const R = this.logoR;
    const pts: number[] = []; // x, y, kind
    const ringStep = 6;
    const ringN = Math.max(24, Math.round((Math.PI * 2 * R) / ringStep));
    for (let i = 0; i < ringN; i++) {
      const a = (i / ringN) * Math.PI * 2;
      pts.push(this.logoX + Math.cos(a) * R, this.logoY + Math.sin(a) * R, KIND_RING);
    }
    const side = R * 2;
    const off = document.createElement("canvas");
    off.width = off.height = side;
    const g = off.getContext("2d", { willReadFrequently: true });
    if (g) {
      g.font = font;
      g.textAlign = "center";
      g.textBaseline = "middle";
      g.fillStyle = "#fff";
      g.fillText("OX", R, R * 1.04);
      const data = g.getImageData(0, 0, side, side).data;
      const step = Math.max(4, Math.round(R / 21));
      let row = 0;
      for (let y = 0; y < side; y += step, row++) {
        // Зсув кожного другого рядка — растр «стільниками», а не квадратна сітка.
        for (let x = row % 2 ? step / 2 : 0; x < side; x += step) {
          if ((data[(Math.floor(y) * side + Math.floor(x)) * 4 + 3] ?? 0) > 140) {
            pts.push(this.logoX - R + x, this.logoY - R + y, KIND_LETTER);
          }
        }
      }
    }
    const logoN = pts.length / 3;
    const dustN = Math.round(logoN * 0.7);
    for (let i = 0; i < dustN; i++) {
      const a = Math.random() * Math.PI * 2;
      const r = R * (1.15 + Math.random() * 0.7);
      pts.push(this.logoX + Math.cos(a) * r, this.logoY + Math.sin(a) * r, KIND_DUST);
    }

    const n = pts.length / 3;
    this.n = n;
    this.kind = new Uint8Array(n);
    this.bucket = new Uint8Array(n);
    this.streak = new Uint8Array(n);
    this.sx = new Float32Array(n);
    this.sy = new Float32Array(n);
    this.mx = new Float32Array(n);
    this.my = new Float32Array(n);
    this.tx = new Float32Array(n);
    this.ty = new Float32Array(n);
    this.ex = new Float32Array(n);
    this.ey = new Float32Array(n);
    this.delay = new Float32Array(n);
    this.size = new Float32Array(n);
    // Фонове коло (як у DotField): центр контентної області, радіус 40 % меншої сторони.
    const contentW = Math.max(1, this.w - LEFT_INSET);
    const bgX = LEFT_INSET + contentW / 2;
    const bgR = Math.min(contentW, this.h) * 0.4;
    for (let i = 0; i < n; i++) {
      const x = pts[i * 3] ?? 0;
      const y = pts[i * 3 + 1] ?? 0;
      const kind = pts[i * 3 + 2] ?? KIND_DUST;
      this.kind[i] = kind;
      this.tx[i] = x;
      this.ty[i] = y;
      const u = clamp01((x - (this.logoX - R)) / (2 * R));
      this.bucket[i] = kind === KIND_DUST ? Math.floor(Math.random() * BUCKETS) : Math.round(u * (BUCKETS - 1));
      this.streak[i] = Math.random() < 0.35 ? 1 : 0;
      // Хмара піднімається знизу.
      const sx = Math.random() * this.w;
      const sy = this.h + 10 + Math.random() * this.h * 0.35;
      this.sx[i] = sx;
      this.sy[i] = sy;
      this.mx[i] = sx + (Math.random() - 0.5) * this.w * 0.15;
      this.my[i] = this.h * (0.3 + Math.random() * 0.55);
      this.delay[i] = Math.random() * GATHER_SPREAD_MS;
      this.size[i] = kind === KIND_LETTER ? 3.4 : kind === KIND_RING ? 3 : 2.2 + Math.random() * 1.6;
      // Кільце розходиться в кільця фонового кола (той самий кут).
      const a = Math.atan2(y - this.logoY, x - this.logoX);
      const ring = 4 + Math.floor(Math.random() * (BG_RINGS - 3));
      this.ex[i] = bgX + Math.cos(a) * bgR * (ring / BG_RINGS);
      this.ey[i] = this.h / 2 + Math.sin(a) * bgR * (ring / BG_RINGS);
    }
  }

  /** Початок виходу: літери — до знака в сайдбарі (або на місці, якщо його немає). */
  private beginOut(t: number): void {
    this.outAt = t;
    this.outMs = this.skipped ? SKIP_MS : OUT_MS;
    this.bg.style.pointerEvents = "none";
    const el = this.markScale > 0 ? document.querySelector<HTMLElement>("[data-splash-mark]") : null;
    const rect = el?.getBoundingClientRect();
    if (rect && rect.width > 0) {
      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;
      this.markScale = rect.width / (2 * this.logoR);
      for (let i = 0; i < this.n; i++) {
        if (this.kind[i] !== KIND_LETTER) continue;
        this.ex[i] = cx + ((this.tx[i] ?? 0) - this.logoX) * this.markScale;
        this.ey[i] = cy + ((this.ty[i] ?? 0) - this.logoY) * this.markScale;
      }
    } else {
      this.markScale = 1;
      this.ex.set(this.tx);
      this.ey.set(this.ty);
    }
    // Контент з'являється разом із розчиненням фону (transition у App.svelte).
    document.documentElement.classList.remove("splashing");
  }

  private frame = (now: number): void => {
    if (this.finished) return;
    if (this.t0 < 0) this.t0 = now;
    let t = now - this.t0;
    if (this.outAt < 0) {
      const holdDone = t >= GATHERED_MS + HOLD_MS;
      if (this.isReady && (this.skipped || holdDone)) this.beginOut(t);
      else if (t >= MAX_WAIT_MS) {
        this.markScale = 0;
        this.skipped = true;
        this.beginOut(t);
      }
    }
    // Пропуск — одразу зібраний знак, щоб вихід не стрибав із хаосу.
    if (this.skipped && this.outAt >= 0 && this.outAt < GATHERED_MS) {
      this.t0 -= GATHERED_MS - this.outAt;
      this.outAt = GATHERED_MS;
      t = now - this.t0;
    }
    const start = performance.now();
    const v = this.outAt >= 0 ? clamp01((t - this.outAt) / this.outMs) : 0;
    this.draw(t, v);
    this.spentMs += performance.now() - start;
    this.frames++;
    if (this.outAt >= 0 && v >= 1) {
      this.finish();
      return;
    }
    this.raf = requestAnimationFrame(this.frame);
  };

  private draw(t: number, v: number): void {
    const ctx = this.ctx;
    if (!ctx) return;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.clearRect(0, 0, this.w, this.h);
    const out = this.outAt >= 0;
    const eOut = easeInOut(v);
    // Поки чекаємо бекенд — легке «дихання» знака; на виході воно завмирає там, де було
    // (інакше на старті польоту знак стрибне на 1–2 %).
    const bt = out ? this.outAt : t;
    const breath = bt > GATHERED_MS ? 1 + 0.015 * Math.sin(((bt - GATHERED_MS) / 1000) * Math.PI) : 1;
    for (let i = 0; i < this.n; i++) {
      const kind = this.kind[i] ?? KIND_DUST;
      const g0 = GATHER_FROM_MS + (this.delay[i] ?? 0);
      const tx = this.logoX + ((this.tx[i] ?? 0) - this.logoX) * breath;
      const ty = this.logoY + ((this.ty[i] ?? 0) - this.logoY) * breath;
      let x: number;
      let y: number;
      let alpha: number;
      let size = this.size[i] ?? 3;
      let streakLen = 0;
      if (t < g0) {
        const e = easeOut(t / g0);
        x = (this.sx[i] ?? 0) + ((this.mx[i] ?? 0) - (this.sx[i] ?? 0)) * e;
        y = (this.sy[i] ?? 0) + ((this.my[i] ?? 0) - (this.sy[i] ?? 0)) * e;
        alpha = 0.2 + 0.4 * e;
        if (this.streak[i]) streakLen = 18 * (1 - e);
      } else if (t < g0 + GATHER_MS) {
        const e = easeInOut((t - g0) / GATHER_MS);
        x = (this.mx[i] ?? 0) + (tx - (this.mx[i] ?? 0)) * e;
        y = (this.my[i] ?? 0) + (ty - (this.my[i] ?? 0)) * e;
        alpha = kind === KIND_DUST ? 0.6 * (1 - e) : 0.6 + 0.3 * e;
      } else {
        x = tx;
        y = ty;
        alpha = kind === KIND_DUST ? 0 : 0.9;
      }
      if (out && kind !== KIND_DUST) {
        x += ((this.ex[i] ?? 0) - x) * eOut;
        y += ((this.ey[i] ?? 0) - y) * eOut;
        if (kind === KIND_LETTER) {
          size *= 1 + (Math.max(0.35, this.markScale) - 1) * eOut;
          alpha *= 1 - clamp01((v - 0.75) / 0.25);
        } else {
          alpha *= 1 - eOut;
        }
      }
      if (alpha <= 0.01) continue;
      const b = this.bucket[i] ?? 0;
      ctx.globalAlpha = alpha;
      if (streakLen > 1) {
        ctx.fillStyle = this.colors[b] ?? "#fff";
        ctx.fillRect(x - 0.5, y, 1, streakLen);
      }
      const sprite = this.sprites[b];
      if (!sprite) continue;
      const s = size * 2.2;
      ctx.drawImage(sprite, x - s / 2, y - s / 2, s, s);
    }
    // Пауза з акцентом: тонка хвиля-кільце розходиться від знака.
    const w = (t - GATHERED_MS) / WAVE_MS;
    if (w > 0 && w < 1 && !out) {
      const e = easeOut(w);
      ctx.globalAlpha = 0.45 * (1 - w);
      ctx.strokeStyle = this.waveColor;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(this.logoX, this.logoY, this.logoR * (1 + 1.3 * e), 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    // Фон розчиняється — під ним уже інтерфейс і фонове коло.
    const op = out ? 1 - eOut : 1;
    if (op !== this.bgOpacity) {
      this.bgOpacity = op;
      this.bg.style.opacity = String(op);
    }
  }

  private finish(): void {
    if (this.finished) return;
    this.finished = true;
    cancelAnimationFrame(this.raf);
    window.removeEventListener("pointerdown", this.skip, true);
    window.removeEventListener("keydown", this.skip, true);
    this.bg.remove();
    this.canvas.remove();
    document.documentElement.classList.remove("splashing", "splash-mark");
    if (this.frames > 0) {
      console.info(
        `[splash] ${this.n} точок, ${this.frames} кадрів, ${(this.spentMs / this.frames).toFixed(2)} мс/кадр`,
      );
    }
  }
}
