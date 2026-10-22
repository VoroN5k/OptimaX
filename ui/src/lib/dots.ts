// Фонова анімація «пульсуюче коло з точок» (CLAUDE.md, розд. 10.4).
//
// Обмеження продуктивності (програма для оптимізації не має сама навантажувати систему):
// - не більше 30 FPS, у спокої — 15 FPS (повільна хвиля
//   й обертання 1–3 °/с на око не відрізняються, а навантаження вдвічі менше). Пробудження —
//   таймером із потрібною частотою, а не requestAnimationFrame на кожен vsync: на моніторі
//   200 Гц це 15 пробуджень замість 200 на секунду;
// - canvas завбільшки з коло, а не на весь екран (менше пікселів для композитингу);
// - жодних алокацій у кадрі: позиції — Float32Array, точки — заздалегідь намальовані спрайти,
//   колір задається вибором спрайта, прозорість — числом globalAlpha;
// - devicePixelRatio обмежено: 1.5 для «Високої», 1 для «Низької» (точки м'які, різниці не видно);
// - якщо середній час кадру > 6 мс — кількість точок автоматично зменшується;
// - зупинка/старт — ззовні (компонент слідкує за видимістю вікна, фокусом, режимом гри тощо).

export type DotsMode = "idle" | "scanning" | "warn" | "ok";
export type DotsQuality = "high" | "low";

interface Params {
  speed: number; // множник швидкості хвилі
  amp: number; // амплітуда зміни розміру точки (0.15–0.3)
  bright: number; // множник яскравості
  warn: number; // «тепла» хвиля
  ok: number; // «зелена» хвиля
}

const TARGET: Record<DotsMode, Params> = {
  idle: { speed: 1, amp: 0.18, bright: 0.85, warn: 0, ok: 0 },
  scanning: { speed: 2.1, amp: 0.28, bright: 1.15, warn: 0, ok: 0 },
  warn: { speed: 1.4, amp: 0.26, bright: 1, warn: 1, ok: 0 },
  ok: { speed: 1.4, amp: 0.26, bright: 1, warn: 0, ok: 1 },
};

const WAVE_PERIOD_S = 5; // період пульсації 4–6 с
const BREATH_PERIOD_S = 8; // «дихання» всього кола ±2 %
const TRANSITION_MS = 450; // плавний перехід між станами (≥ 400 мс)
const PARALLAX_PX = 8; // параллакс від курсора ≤ 8 px
const FPS_ACTIVE = 30;
const FPS_IDLE = 15;
/** Запас навколо кола: параллакс, «дихання» і м'яке світіння точок. */
const CANVAS_MARGIN = 1.18;
const SLOW_FRAME_MS = 6;
const BUCKETS = 16; // відтінки градієнта (спрайти)
const SPRITE_PX = 32;

function hexToRgb(hex: string): [number, number, number] {
  const h = hex.trim().replace("#", "");
  const n = parseInt(h.length === 3 ? h.replace(/(.)/g, "$1$1") : h, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function makeSprite(rgb: [number, number, number], px: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = c.height = px;
  const g = c.getContext("2d");
  if (!g) return c;
  const grad = g.createRadialGradient(px / 2, px / 2, 0, px / 2, px / 2, px / 2);
  const [r, gg, b] = rgb;
  grad.addColorStop(0, `rgba(${Math.min(255, r + 70)},${Math.min(255, gg + 70)},${Math.min(255, b + 70)},1)`);
  grad.addColorStop(0.35, `rgba(${r},${gg},${b},0.9)`);
  grad.addColorStop(1, `rgba(${r},${gg},${b},0)`);
  g.fillStyle = grad;
  g.fillRect(0, 0, px, px);
  return c;
}

export interface DotsStats {
  dots: number;
  avgFrameMs: number;
  fps: number;
}

export class DotField {
  private ctx: CanvasRenderingContext2D | null;
  private dpr = 1;
  private w = 0;
  private h = 0;
  private cx = 0;
  private cy = 0;
  private radius = 0;
  private rings = 0;
  private count = 0;
  private maxDots = 1200;
  private quality: DotsQuality = "high";
  // Дані точок (довжина = count): радіус (0..1), базовий кут, кільце, відро кольору, базова прозорість.
  private rNorm = new Float32Array(0);
  private angle = new Float32Array(0);
  private ring = new Float32Array(0);
  private bucket = new Uint8Array(0);
  private alpha = new Float32Array(0);
  private sprites: HTMLCanvasElement[] = [];
  private warnSprite: HTMLCanvasElement | null = null;
  private okSprite: HTMLCanvasElement | null = null;
  private cur: Params = { ...TARGET.idle };
  private target: Params = { ...TARGET.idle };
  private running = false;
  private raf = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private last = 0;
  private t = 0;
  private mouseX = 0;
  private mouseY = 0;
  private parX = 0;
  private parY = 0;
  private emaMs = 0;
  private slowFrames = 0;
  private frames = 0;
  private fpsWindowStart = 0;
  private fps = 0;
  private fpsLimit = FPS_IDLE;

  constructor(
    private canvas: HTMLCanvasElement,
    private leftInset: number,
  ) {
    this.ctx = canvas.getContext("2d", { alpha: true });
    const css = getComputedStyle(document.documentElement);
    const from = hexToRgb(css.getPropertyValue("--accent-from") || "#6E7BFF");
    const to = hexToRgb(css.getPropertyValue("--accent-to") || "#8B5CF6");
    const px = SPRITE_PX * 2;
    for (let i = 0; i < BUCKETS; i++) {
      // Градієнт за кутом: from → to → from (без різкого шва).
      const k = 1 - Math.abs((i / (BUCKETS - 1)) * 2 - 1);
      const rgb: [number, number, number] = [
        Math.round(from[0] + (to[0] - from[0]) * k),
        Math.round(from[1] + (to[1] - from[1]) * k),
        Math.round(from[2] + (to[2] - from[2]) * k),
      ];
      this.sprites.push(makeSprite(rgb, px));
    }
    this.warnSprite = makeSprite(hexToRgb(css.getPropertyValue("--warn") || "#F5B84B"), px);
    this.okSprite = makeSprite(hexToRgb(css.getPropertyValue("--ok") || "#4ADE80"), px);
  }

  setQuality(q: DotsQuality): void {
    if (q === this.quality) return;
    this.quality = q;
    this.maxDots = q === "high" ? 1200 : 600;
    this.resize();
  }

  setMode(mode: DotsMode): void {
    this.target = { ...TARGET[mode] };
    this.fpsLimit = mode === "idle" ? FPS_IDLE : FPS_ACTIVE;
  }

  setMouse(x: number, y: number): void {
    // Нормалізуємо до -1..1 відносно центру кола на екрані.
    this.mouseX = Math.max(-1, Math.min(1, (x - this.screenCx) / (this.w / 2 || 1)));
    this.mouseY = Math.max(-1, Math.min(1, (y - this.screenCy) / (this.h / 2 || 1)));
  }

  private screenCx = 0;
  private screenCy = 0;

  resize(): void {
    const maxDpr = this.quality === "high" ? 1.5 : 1;
    this.dpr = Math.min(maxDpr, window.devicePixelRatio || 1);
    this.w = window.innerWidth;
    this.h = window.innerHeight;
    const contentW = Math.max(1, this.w - this.leftInset);
    const radiusCss = Math.min(contentW, this.h) * 0.4; // 35–45 % меншої сторони
    const side = Math.ceil(radiusCss * 2 * CANVAS_MARGIN + PARALLAX_PX * 2);
    // Центр — посередині контентної області, не під сайдбаром.
    this.screenCx = this.leftInset + contentW / 2;
    this.screenCy = this.h / 2;
    const st = this.canvas.style;
    st.width = `${side}px`;
    st.height = `${side}px`;
    st.left = `${Math.round(this.screenCx - side / 2)}px`;
    st.top = `${Math.round(this.screenCy - side / 2)}px`;
    this.canvas.width = Math.round(side * this.dpr);
    this.canvas.height = Math.round(side * this.dpr);
    this.layout();
    if (!this.running) this.draw(0);
  }

  /** Розкладка точок по концентричних кільцях (алокації — лише тут, не в кадрі). */
  private layout(): void {
    const contentW = Math.max(1, this.w - this.leftInset);
    const minSide = Math.min(contentW, this.h);
    this.radius = minSide * 0.4 * this.dpr;
    this.cx = this.canvas.width / 2;
    this.cy = this.canvas.height / 2;
    const area = contentW * this.h;
    const wanted = Math.round(Math.max(300, Math.min(this.maxDots, area / 1100)));
    this.rings = this.quality === "high" ? 16 : 11;
    // Точки пропорційні довжині кола: n_i ∝ (i + 1).
    const sumWeights = (this.rings * (this.rings + 1)) / 2;
    const perUnit = wanted / sumWeights;
    let total = 0;
    const perRing: number[] = [];
    for (let i = 0; i < this.rings; i++) {
      const n = Math.max(6, Math.round(perUnit * (i + 1)));
      perRing.push(n);
      total += n;
    }
    this.count = total;
    this.rNorm = new Float32Array(total);
    this.angle = new Float32Array(total);
    this.ring = new Float32Array(total);
    this.bucket = new Uint8Array(total);
    this.alpha = new Float32Array(total);
    let k = 0;
    for (let i = 0; i < this.rings; i++) {
      const n = perRing[i] ?? 0;
      const r = (i + 1) / this.rings;
      const offset = i * 0.37;
      for (let j = 0; j < n; j++, k++) {
        const a = (j / n) * Math.PI * 2 + offset;
        this.rNorm[k] = r;
        this.angle[k] = a;
        this.ring[k] = i;
        const u = (((a % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2)) / (Math.PI * 2);
        this.bucket[k] = Math.min(BUCKETS - 1, Math.floor(u * BUCKETS));
        // Зовнішні кільця прозоріші: 0.7 → 0.15.
        this.alpha[k] = 0.7 - 0.55 * Math.pow(r, 1.3);
      }
    }
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.last = 0;
    this.raf = requestAnimationFrame(this.loop);
  }

  stop(): void {
    this.running = false;
    clearTimeout(this.timer);
    cancelAnimationFrame(this.raf);
  }

  /** Наступний кадр: чекаємо таймером до потрібного моменту, малюємо — у vsync. */
  private schedule(now: number): void {
    const wait = Math.max(0, 1000 / this.fpsLimit - (performance.now() - now) - 2);
    this.timer = setTimeout(this.wake, wait);
  }

  private wake = (): void => {
    if (this.running) this.raf = requestAnimationFrame(this.loop);
  };

  /** Статичний кадр (prefers-reduced-motion або вимкнена анімація при показі). */
  renderStatic(): void {
    this.stop();
    this.cur = { ...TARGET.idle, amp: 0 };
    this.draw(0);
  }

  stats(): DotsStats {
    return { dots: this.count, avgFrameMs: this.emaMs, fps: this.fps };
  }

  private loop = (now: number): void => {
    if (!this.running) return;
    const dt = this.last === 0 ? 1000 / this.fpsLimit : Math.min(100, now - this.last);
    this.last = now;

    // Плавний перехід параметрів (експоненційне згладжування, ~TRANSITION_MS).
    const k = 1 - Math.exp(-dt / (TRANSITION_MS / 3));
    const c = this.cur;
    const g = this.target;
    c.speed += (g.speed - c.speed) * k;
    c.amp += (g.amp - c.amp) * k;
    c.bright += (g.bright - c.bright) * k;
    c.warn += (g.warn - c.warn) * k;
    c.ok += (g.ok - c.ok) * k;
    this.t += (dt / 1000) * c.speed;
    this.parX += (this.mouseX * PARALLAX_PX - this.parX) * k;
    this.parY += (this.mouseY * PARALLAX_PX - this.parY) * k;

    const t0 = performance.now();
    this.draw(this.t);
    const spent = performance.now() - t0;
    this.emaMs = this.emaMs === 0 ? spent : this.emaMs * 0.95 + spent * 0.05;
    this.trackFps(now);
    this.adaptQuality();
    this.schedule(now);
  };

  private trackFps(now: number): void {
    this.frames++;
    if (this.fpsWindowStart === 0) this.fpsWindowStart = now;
    const span = now - this.fpsWindowStart;
    if (span >= 1000) {
      this.fps = (this.frames * 1000) / span;
      this.frames = 0;
      this.fpsWindowStart = now;
    }
  }

  /** Повільні кадри → менше точок (не нижче 300). */
  private adaptQuality(): void {
    if (this.emaMs > SLOW_FRAME_MS) {
      this.slowFrames++;
      if (this.slowFrames > 60 && this.maxDots > 300) {
        this.maxDots = Math.max(300, Math.round(this.count * 0.7));
        this.slowFrames = 0;
        this.emaMs = 0;
        this.layout();
      }
    } else {
      this.slowFrames = 0;
    }
  }

  private draw(t: number): void {
    const ctx = this.ctx;
    if (!ctx) return;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    const c = this.cur;
    const breath = 1 + 0.02 * Math.sin((Math.PI * 2 * t) / BREATH_PERIOD_S);
    const phase = (Math.PI * 2 * t) / WAVE_PERIOD_S;
    const base = (this.quality === "high" ? 3.2 : 3.6) * this.dpr;
    const deg = Math.PI / 180;
    const R = this.radius * breath;
    const px = this.parX * this.dpr;
    const py = this.parY * this.dpr;
    const tintWarn = c.warn > 0.01;
    const tintOk = c.ok > 0.01;
    for (let i = 0; i < this.count; i++) {
      const r = this.rNorm[i] ?? 0;
      const ringIdx = this.ring[i] ?? 0;
      // Обертання кілець у різні боки, 1–3 °/с.
      const dir = ringIdx % 2 === 0 ? 1 : -1;
      const a = (this.angle[i] ?? 0) + dir * (1 + (ringIdx % 3)) * deg * t;
      // Хвиля від центру назовні.
      const wave = Math.sin(phase - r * Math.PI * 2.4);
      const size = base * (1 + c.amp * wave) * (0.8 + 0.4 * (1 - r));
      const depth = 0.3 + 0.7 * r; // зовнішні кільця зсуваються сильніше
      const x = this.cx + Math.cos(a) * r * R + px * depth;
      const y = this.cy + Math.sin(a) * r * R + py * depth;
      const alpha = Math.min(0.7, (this.alpha[i] ?? 0) * c.bright * (0.78 + 0.22 * wave));
      const sprite = this.sprites[this.bucket[i] ?? 0];
      if (!sprite) continue;
      const s = size * 2.2;
      ctx.globalAlpha = alpha;
      ctx.drawImage(sprite, x - s / 2, y - s / 2, s, s);
      if ((tintWarn || tintOk) && wave > 0) {
        const tint = tintWarn ? this.warnSprite : this.okSprite;
        const amount = tintWarn ? c.warn : c.ok;
        if (tint) {
          ctx.globalAlpha = alpha * amount * wave;
          ctx.drawImage(tint, x - s / 2, y - s / 2, s, s);
        }
      }
    }
    ctx.globalAlpha = 1;
  }
}
