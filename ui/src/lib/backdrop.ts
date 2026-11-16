// Фоновий шар «Pro Premium» (docs/DESIGN.md розд. 9 і 11, CLAUDE.md 10.4). Без залежностей.
//
// Два джерела, і жодне не видає випадкові числа за справжні:
// - **запис** (`buildBackdropData`): доріжки — реальні стани головного потоку гри й потоку рендера
//   (точні проміжки біля ривків, решта — частки по 250 мс), смуга кадрів — реальні часи кадрів,
//   помаранчеві — ривки, які визначив бекенд; курсор — повільне відтворення запису (0.05×).
//   Мандрівника немає: запис не містить, на якому ядрі працював потік.
// - **декоративний** фон без запису: доріжки й кадри з детермінованого генератора, курсор і
//   «мандрівник»; підписів і цифр немає (їх малює компонент лише для запису).
//
// Обмеження: ≤ 30 кадрів/с (кадри керує компонент), без алокацій у кадрі — сегменти в типізованих
// масивах, кольори й градієнт готуються при зміні розміру, прозорість через `globalAlpha`.
import type { TimelineView } from "./bindings/TimelineView";

/** Стан сегмента доріжки: працював / чекав на ядро / не працював (сон, синхронізація, диск, інше). */
export const RUN = 0;
export const WAIT = 1;
export const IDLE = 2;

/** Доріжка запису: відрізки, відсортовані за початком (мс від старту запису). */
export interface DataLane {
  key: "main" | "render";
  start: Float64Array;
  dur: Float32Array;
  state: Uint8Array;
  /** Найдовший відрізок — межа для пошуку видимих. */
  maxDur: number;
}

/** Дані фону з запису. */
export interface BackdropData {
  lanes: DataLane[];
  /** Кадри: кінець (Present), тривалість, чи ривок. */
  frameEnd: Float64Array;
  frameFt: Float32Array;
  frameSpike: Uint8Array;
  startMs: number;
  endMs: number;
}

/** Стан потоку з кошика/проміжку (`ThreadLane`: працює, чекає на ядро, синхр., диск, сон, інше). */
function mapState(s: number): number {
  return s === 0 ? RUN : s === 1 ? WAIT : IDLE;
}

type Seg = [number, number, number];

/** Відрізки доріжки: точні проміжки, а кошики без них — частками (працював, чекав, решта). */
function laneSegments(lane: NonNullable<TimelineView["main"]>): Seg[] {
  const bm = lane.bucket_ms;
  const covered = new Uint8Array(lane.buckets.length);
  const out: Seg[] = [];
  for (const [s, d, st] of lane.detail) {
    if (d <= 0) continue;
    out.push([s, d, mapState(st)]);
    if (bm <= 0) continue;
    const a = Math.max(0, Math.floor((s - lane.start_ms) / bm));
    const b = Math.min(covered.length - 1, Math.floor((s + d - lane.start_ms) / bm));
    for (let i = a; i <= b; i++) covered[i] = 1;
  }
  lane.buckets.forEach((k, i) => {
    if (covered[i] || bm <= 0) return;
    const total = k.reduce((a, b) => a + b, 0);
    if (total <= 0) return;
    // Частки кошика розкладаються на його ширину: спершу «працював», потім «чекав», потім решта.
    const scale = bm / total;
    let t = lane.start_ms + i * bm;
    const parts: [number, number][] = [
      [k[0], RUN],
      [k[1], WAIT],
      [k[2] + k[3] + k[4] + k[5], IDLE],
    ];
    for (const [ms, st] of parts) {
      if (ms <= 0) continue;
      out.push([t, ms * scale, st]);
      t += ms * scale;
    }
  });
  out.sort((a, b) => a[0] - b[0]);
  // Сусідні відрізки одного стану зливаються — менше прямокутників у кадрі.
  const merged: Seg[] = [];
  for (const s of out) {
    const last = merged[merged.length - 1];
    if (last && last[2] === s[2] && Math.abs(last[0] + last[1] - s[0]) < 0.5) last[1] = s[0] + s[1] - last[0];
    else merged.push([s[0], s[1], s[2]]);
  }
  return merged;
}

/** Будує дані фону з шкали запису. `null` — у записі немає станів потоків чи кадрів. */
export function buildBackdropData(view: TimelineView | null | undefined): BackdropData | null {
  if (!view || view.frames.t.length < 2) return null;
  const lanes: DataLane[] = [];
  for (const key of ["main", "render"] as const) {
    const l = view[key];
    if (!l) continue;
    const segs = laneSegments(l);
    if (segs.length === 0) continue;
    lanes.push({
      key,
      start: Float64Array.from(segs, (s) => s[0]),
      dur: Float32Array.from(segs, (s) => s[1]),
      state: Uint8Array.from(segs, (s) => s[2]),
      maxDur: segs.reduce((m, s) => Math.max(m, s[1]), 0),
    });
  }
  if (lanes.length === 0) return null;
  const n = view.frames.t.length;
  const frameEnd = Float64Array.from(view.frames.t);
  const frameFt = Float32Array.from(view.frames.ft);
  const frameSpike = new Uint8Array(n);
  // Ривки — з бекенду (час Present збігається з часом кадру); обидва списки відсортовані.
  let j = 0;
  for (const [t] of view.spikes) {
    while (j < n && (frameEnd[j] ?? 0) < t - 0.01) j++;
    if (j < n && Math.abs((frameEnd[j] ?? 0) - t) <= 0.01) frameSpike[j] = 1;
  }
  return { lanes, frameEnd, frameFt, frameSpike, startMs: view.start_ms, endMs: view.end_ms };
}

/** Перший індекс `i`, де `arr[i] >= v` (масив відсортований). */
export function lowerBound(arr: Float64Array, v: number): number {
  let lo = 0;
  let hi = arr.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if ((arr[mid] ?? 0) < v) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** Детермінований генератор (mulberry32): однаковий вигляд декоративного фону щоразу. */
export function rng(seed: number): () => number {
  let s = seed | 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function hash(a: number, b: number): number {
  const x = Math.sin(a * 127.1 + b * 311.7) * 43758.5453;
  return x - Math.floor(x);
}

const clamp = (v: number, a: number, b: number) => (v < a ? a : v > b ? b : v);
const easeOutCubic = (k: number) => 1 - Math.pow(1 - clamp(k, 0, 1), 3);

/** Геометрія (docs/DESIGN.md розд. 9). */
export const LANE_H = 8;
const LANE_STEP = 56;
const TOP = 104;
const BOTTOM = 200;
const STRIP_FROM_BOTTOM = 168;
const STRIP_H = 16;
const SLOT = 96;
const SLOT_SPEED = 26;
const CURSOR_CYCLE = 18;
const CURSOR_SWEEP = 13;
const CURSOR_DELAY = 1.2;
const NEAR = 150;
/** Запис: пікселів на мілісекунду й швидкість відтворення. */
const DATA_PX_PER_MS = 0.4;
const DATA_RATE = 0.05;
const DATA_TAIL_MS = 4000;
const SEG_CAP = 256;
const TRAIL_CAP = 32;

/** Базова прозорість станів (працював / чекав / не працював). */
const ALPHA = [0.3, 0.55, 0.13] as const;

/** Кольори з токенів (`r,g,b`): компонент читає їх із CSS. */
export interface BackdropColors {
  run: string;
  wait: string;
  idle: string;
  frame: string;
  over: string;
  cursor: string;
}

/** Геометрія доріжок для шару підписів. */
export interface LaneLayout {
  y: number[];
  stripY: number;
}

class DecoLane {
  y = 0;
  speed: number;
  phase: number;
  r: () => number;
  x = new Float64Array(SEG_CAP);
  len = new Float32Array(SEG_CAP);
  st = new Uint8Array(SEG_CAP);
  head = 0;
  count = 0;
  end = 0;
  last = WAIT;

  constructor(i: number) {
    this.r = rng(1000 + i * 17);
    this.speed = 10 + this.r() * 12;
    this.phase = this.r() * 900;
  }

  /** Добудовує сегменти до `upto` і відкидає ті, що вже далеко ліворуч (без алокацій). */
  extend(upto: number, keepFrom: number): void {
    while (this.count > 0) {
      const h = this.head;
      if ((this.x[h] ?? 0) + (this.len[h] ?? 0) >= keepFrom) break;
      this.head = (h + 1) % SEG_CAP;
      this.count--;
    }
    while (this.end < upto && this.count < SEG_CAP) {
      const st = this.last === RUN ? (this.r() < 0.09 ? WAIT : IDLE) : RUN;
      this.last = st;
      const len = st === RUN ? 40 + this.r() * 190 : st === WAIT ? 14 + this.r() * 40 : 18 + this.r() * 110;
      const k = (this.head + this.count) % SEG_CAP;
      this.x[k] = this.end;
      this.len[k] = len;
      this.st[k] = st;
      this.count++;
      this.end += len + (st === RUN ? 3 : 0);
    }
  }
}

/** Малювання анімованого шару. Компонент викликає `frame(now)` не частіше 30 разів на секунду. */
export class BackdropScene {
  private ctx: CanvasRenderingContext2D;
  private w = 0;
  private h = 0;
  private data: BackdropData | null = null;
  private colors: BackdropColors;
  private fill: Record<keyof BackdropColors, string>;
  private stateFill: string[] = [];
  private deco: DecoLane[] = [];
  private dataY: number[] = [];
  private y0 = TOP;
  private y1 = 0;
  private stripY = 0;
  private cursorGrad: CanvasGradient | null = null;
  private t0 = -1;
  // Мандрівник (лише декоративний фон): поточна ціль, звідки летить, час стрибка; слід — кільце.
  private hot = { on: false, x: 0, y: 0, fx: 0, fy: 0, ts: 0 };
  private armed = false;
  private trailX = new Float32Array(TRAIL_CAP);
  private trailY = new Float32Array(TRAIL_CAP);
  private trailT = new Float64Array(TRAIL_CAP);
  private trailHead = 0;
  private trailN = 0;

  constructor(ctx: CanvasRenderingContext2D, colors: BackdropColors) {
    this.ctx = ctx;
    this.colors = colors;
    this.fill = colors;
    this.setColors(colors);
  }

  /** Кольори з токенів (при зміні палітри). */
  setColors(c: BackdropColors): void {
    this.colors = c;
    const rgb = (v: string) => `rgb(${v})`;
    this.fill = {
      run: rgb(c.run),
      wait: rgb(c.wait),
      idle: rgb(c.idle),
      frame: rgb(c.frame),
      over: rgb(c.over),
      cursor: rgb(c.cursor),
    };
    this.stateFill = [this.fill.run, this.fill.wait, this.fill.idle];
    this.makeGradient();
  }

  /** Дані запису або `null` — декоративний фон. Перезапускає відлік часу. */
  setData(d: BackdropData | null): void {
    this.data = d;
    this.t0 = -1;
    this.layout(this.w, this.h);
  }

  /** Чи показує фон реальний запис. */
  get real(): boolean {
    return this.data !== null;
  }

  /** Розмір у CSS-пікселях; повертає положення доріжок для шару підписів. */
  layout(w: number, h: number): LaneLayout {
    this.w = w;
    this.h = h;
    this.y0 = TOP;
    this.y1 = Math.max(TOP + LANE_STEP, h - BOTTOM);
    this.stripY = h - STRIP_FROM_BOTTOM;
    if (this.data) {
      const n = this.data.lanes.length;
      this.dataY = this.data.lanes.map((_, i) => this.y0 + ((this.y1 - this.y0) * (i + 1)) / (n + 1));
    } else {
      const n = clamp(Math.round((this.y1 - this.y0) / LANE_STEP) + 1, 6, 28);
      if (this.deco.length !== n) this.deco = Array.from({ length: n }, (_, i) => new DecoLane(i));
      this.deco.forEach((l, i) => (l.y = this.y0 + (i * (this.y1 - this.y0)) / (n - 1)));
    }
    this.makeGradient();
    return { y: this.data ? this.dataY : this.deco.map((l) => l.y), stripY: this.stripY };
  }

  private makeGradient(): void {
    const g = this.ctx.createLinearGradient(-100, 0, 2, 0);
    g.addColorStop(0, `rgba(${this.colors.cursor},0)`);
    g.addColorStop(1, `rgba(${this.colors.cursor},0.10)`);
    this.cursorGrad = g;
  }

  /** Один кадр. `animated = false` — статичний кадр (зменшений рух): без курсора й мандрівника. */
  frame(now: number, animated: boolean): void {
    if (this.t0 < 0) this.t0 = now;
    const t = animated ? (now - this.t0) / 1000 : 8;
    this.ctx.clearRect(0, 0, this.w, this.h);
    if (this.data) this.drawData(this.data, t, animated);
    else this.drawDeco(now, t, animated);
    this.ctx.globalAlpha = 1;
  }

  private rect(x: number, y: number, w: number, h: number): void {
    // roundRect дорожчий і на 8 px висоти майже не помітний — звичайні прямокутники.
    this.ctx.fillRect(x, y, w, h);
  }

  private drawCursor(wx: number): void {
    const ctx = this.ctx;
    const top = this.y0 - 24;
    const height = this.y1 - this.y0 + 248;
    if (this.cursorGrad) {
      ctx.globalAlpha = 1;
      ctx.translate(wx, 0);
      ctx.fillStyle = this.cursorGrad;
      ctx.fillRect(-100, top, 102, height);
      ctx.translate(-wx, 0);
    }
    ctx.globalAlpha = 0.42;
    ctx.fillStyle = this.fill.cursor;
    ctx.fillRect(wx, top, 1, height);
  }

  private drawData(d: BackdropData, t: number, animated: boolean): void {
    const ctx = this.ctx;
    const w = this.w;
    const cx = Math.round(w * 0.5);
    // Відтворення: від моменту, коли ліва половина вже заповнена, до кінця запису й паузи.
    const from = d.startMs + cx / DATA_PX_PER_MS;
    const span = Math.max(1, d.endMs - from) + DATA_TAIL_MS;
    const T = from + ((t * 1000 * DATA_RATE) % span);
    const tl = T - cx / DATA_PX_PER_MS;
    const tr = T + (w - cx) / DATA_PX_PER_MS;
    const rv = animated ? clamp(t / 0.5, 0, 1) : 1;
    for (let i = 0; i < d.lanes.length; i++) {
      const L = d.lanes[i];
      const y = this.dataY[i] ?? 0;
      if (!L) continue;
      ctx.globalAlpha = 0.03 * rv;
      ctx.fillStyle = "#fff";
      ctx.fillRect(0, y + LANE_H / 2, w, 1);
      for (let k = lowerBound(L.start, tl - L.maxDur); k < L.start.length; k++) {
        const s = L.start[k] ?? 0;
        if (s > tr) break;
        const len = (L.dur[k] ?? 0) * DATA_PX_PER_MS;
        const x = cx + (s - T) * DATA_PX_PER_MS;
        if (x + len < 0) continue;
        const st = L.state[k] ?? IDLE;
        let a = ALPHA[st] ?? 0.13;
        if (animated) {
          const dx = Math.abs(x + len / 2 - cx);
          if (dx < NEAR) a += (st === IDLE ? 0.08 : 0.18) * (1 - dx / NEAR);
        }
        ctx.globalAlpha = a * rv;
        ctx.fillStyle = this.stateFill[st] ?? this.fill.idle;
        this.rect(x, y, Math.max(1, len - 0.5), LANE_H);
      }
    }
    // Кадри на тій самій осі часу: ширина — тривалість кадру, ривки помаранчеві.
    const fy = this.stripY;
    for (let k = lowerBound(d.frameEnd, tl); k < d.frameEnd.length; k++) {
      const end = d.frameEnd[k] ?? 0;
      const ft = d.frameFt[k] ?? 0;
      if (end - ft > tr) break;
      const x = cx + (end - ft - T) * DATA_PX_PER_MS;
      const fw = Math.max(1, ft * DATA_PX_PER_MS - 1);
      const spike = d.frameSpike[k] === 1;
      ctx.globalAlpha = (spike ? 0.7 : 0.34) * rv;
      ctx.fillStyle = spike ? this.fill.over : this.fill.frame;
      this.rect(x, spike ? fy - 6 : fy, fw, spike ? STRIP_H + 6 : STRIP_H);
    }
    if (animated) this.drawCursor(cx);
  }

  private pushTrail(x: number, y: number, ts: number): void {
    const k = (this.trailHead + this.trailN) % TRAIL_CAP;
    this.trailX[k] = x;
    this.trailY[k] = y;
    this.trailT[k] = ts;
    if (this.trailN < TRAIL_CAP) this.trailN++;
    else this.trailHead = (this.trailHead + 1) % TRAIL_CAP;
  }

  private hop(x: number, y: number, now: number): void {
    const H = this.hot;
    if (H.on) {
      const k = easeOutCubic((now - H.ts) / 280);
      const cx = H.fx + (H.x - H.fx) * k;
      const cy = H.fy + (H.y - H.fy) * k;
      this.pushTrail(cx, cy, now);
      H.fx = cx;
      H.fy = cy;
    } else {
      H.fx = x;
      H.fy = y;
      H.on = true;
    }
    H.x = x;
    H.y = y;
    H.ts = now;
  }

  private randomLaneY(): number {
    return this.deco[Math.floor(Math.random() * this.deco.length)]?.y ?? this.y0;
  }

  private drawDeco(now: number, t: number, animated: boolean): void {
    const ctx = this.ctx;
    const w = this.w;
    let wx = -1;
    if (animated && t > CURSOR_DELAY) {
      const ph = (t - CURSOR_DELAY) % CURSOR_CYCLE;
      if (ph < CURSOR_SWEEP) {
        wx = -120 + (w + 240) * (ph / CURSOR_SWEEP);
        if (!this.armed) {
          this.armed = true;
          this.hot.on = false;
          this.trailN = 0;
          this.hop(w * (0.06 + Math.random() * 0.14), this.randomLaneY(), now);
        }
      } else {
        this.armed = false;
        this.hot.on = false;
      }
    }
    for (let i = 0; i < this.deco.length; i++) {
      const L = this.deco[i];
      if (!L) continue;
      const rv = animated ? clamp((t - i * 0.03) / 0.5, 0, 1) : 1;
      const off = L.phase + (animated ? t * L.speed : 0);
      L.extend(off + w + 60, off - 400);
      ctx.globalAlpha = 0.03 * rv;
      ctx.fillStyle = "#fff";
      ctx.fillRect(0, L.y + LANE_H / 2, w, 1);
      for (let c = 0; c < L.count; c++) {
        const k = (L.head + c) % SEG_CAP;
        const x = (L.x[k] ?? 0) - off;
        const len = L.len[k] ?? 0;
        if (x > w) break;
        if (x + len < 0) continue;
        const st = L.st[k] ?? IDLE;
        let a = ALPHA[st] ?? 0.13;
        if (wx >= -120) {
          const dx = Math.abs(x + len / 2 - wx);
          if (dx < NEAR) a += (st === IDLE ? 0.08 : 0.18) * (1 - dx / NEAR);
        }
        ctx.globalAlpha = a * rv;
        ctx.fillStyle = this.stateFill[st] ?? this.fill.idle;
        this.rect(x, L.y, len, LANE_H);
      }
    }
    // Бюджет кадру (декоративний): слоти прокручуються, зрідка кадр вилазить за слот.
    const fy = this.stripY;
    const rv2 = animated ? clamp((t - 0.7) / 0.5, 0, 1) : 1;
    const foff = animated ? t * SLOT_SPEED : 0;
    const k1 = Math.ceil((foff + w) / SLOT) + 1;
    for (let k = Math.floor(foff / SLOT) - 1; k <= k1; k++) {
      const sx = k * SLOT - foff;
      const spike = hash(k, 9) > 0.9;
      const f = spike ? 1.4 + hash(k, 5) * 0.35 : 0.68 + hash(k, 3) * 0.26;
      ctx.globalAlpha = 0.06 * rv2;
      ctx.fillStyle = "#fff";
      ctx.fillRect(sx, fy - 4, 1, STRIP_H + 8);
      ctx.globalAlpha = 0.34 * rv2;
      ctx.fillStyle = this.fill.frame;
      this.rect(sx + 3, fy, Math.min(f, 1) * SLOT - 6, STRIP_H);
      if (f > 1) {
        ctx.globalAlpha = 0.7 * rv2;
        ctx.fillStyle = this.fill.over;
        this.rect(sx + SLOT - 3, fy, (f - 1) * SLOT, STRIP_H);
      }
    }
    if (wx >= -120) this.drawCursor(wx);
    if (!animated) return;
    // Слід мандрівника тьмяніє 1.8 с.
    for (let c = 0; c < this.trailN; c++) {
      const k = (this.trailHead + c) % TRAIL_CAP;
      const age = (now - (this.trailT[k] ?? 0)) / 1800;
      if (age >= 1) continue;
      ctx.globalAlpha = 0.55 * (1 - age) * (1 - age);
      ctx.fillStyle = this.fill.wait;
      this.rect(this.trailX[k] ?? 0, (this.trailY[k] ?? 0) - 1, 72, LANE_H + 2);
    }
    const H = this.hot;
    if (!H.on) return;
    if (wx >= H.x - 2) {
      const nx = wx + 150 + Math.random() * 200;
      if (nx < w - 90) this.hop(nx + Math.random() * 40, this.randomLaneY(), now);
      else {
        H.on = false;
        return;
      }
    }
    const k = easeOutCubic((now - H.ts) / 280);
    const x = H.fx + (H.x - H.fx) * k;
    const y = H.fy + (H.y - H.fy) * k;
    ctx.globalAlpha = 0.16;
    ctx.fillStyle = this.fill.over;
    this.rect(x - 5, y - 5, 82, LANE_H + 10);
    ctx.globalAlpha = 0.95;
    ctx.fillStyle = this.fill.wait;
    this.rect(x, y - 1, 72, LANE_H + 2);
  }
}

/** `#RRGGBB` → `r,g,b` (для кольорів з токенів). */
export function hexToTriplet(hex: string, fallback: string): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m || !m[1]) return fallback;
  const v = parseInt(m[1], 16);
  return `${(v >> 16) & 255},${(v >> 8) & 255},${v & 255}`;
}
