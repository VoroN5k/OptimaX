// Фоновий шар «Шовк» (docs/DESIGN.md розд. 9, CLAUDE.md 10.4). Без залежностей.
//
// Суто декоративний: тонкі лінії поверхні, що повільно «дихає», у перспективі. Ліній однаково
// багато, а яскравість дає сама геометрія: де поверхня нахиляється до глядача, лінії сходяться
// щільніше й виблискують, як шовк. Жодних даних, підписів чи цифр.
//
// Обмеження: частоту кадрів задає компонент; у кадрі без алокацій — точки в типізованих масивах,
// кольори рядків готуються при зміні розміру чи палітри.

/** Кольори з токенів (`r,g,b`): компонент читає їх із CSS. */
export interface BackdropColors {
  /** Ближні лінії (акцент). */
  near: string;
  /** Дальні лінії (холодний сірий). */
  far: string;
}

/** Сітка: рядки вглиб × точки вздовж рядка. */
const ROWS = 64;
const COLS = 96;
/** Поверхня в світових одиницях: ширина, глибина, відстань від камери до центру, висота камери. */
const WIDTH = 30;
const DEPTH = 14;
const DIST = 10.5;
const CAM_H = 4;
/** Поворот поверхні (рад) і його повільне хитання — дає відчуття об'єму. */
const YAW = 0.26;
const YAW_SWAY = 0.045;
/** Найближча допустима глибина: ближче — точка не малюється. */
const NEAR_Z = 0.8;
/** Найбільша прозорість ближнього рядка й дальнього (ближні все одно здебільшого під плитками). */
const ALPHA_NEAR = 0.2;
const ALPHA_FAR = 0.06;

/** Точок у шматку рядка зі спільною яскравістю; крок сітки в світових одиницях. */
const CHUNK = 6;
const Z_STEP = DEPTH / (ROWS - 1);
const X_STEP = WIDTH / (COLS - 1);
/** Яскравість шматка: тінь і полиск (множник до прозорості рядка). */
const SHADE_MIN = 0.3;
const SHADE_GAIN = 3.2;

const clamp = (v: number, a: number, b: number) => (v < a ? a : v > b ? b : v);
const smooth = (a: number, b: number, v: number) => {
  const k = clamp((v - a) / (b - a), 0, 1);
  return k * k * (3 - 2 * k);
};

/**
 * Висота поверхні в точці (`x`, `z` — локальні координати, `t` — секунди). Кілька пологих хвиль з
 * різними напрямками й несумірними швидкостями: складки не повторюються, період найшвидшої ≈ 40 с.
 */
export function height(x: number, z: number, t: number): number {
  return (
    0.9 * Math.sin(0.34 * x + 0.21 * z + 0.105 * t) +
    0.55 * Math.sin(-0.23 * x + 0.47 * z - 0.071 * t + 1.3) +
    0.25 * Math.sin(0.71 * x - 0.36 * z + 0.157 * t + 2.1) +
    0.45 * Math.sin(0.12 * x + 0.083 * t + 0.4)
  );
}

/** Малювання шару. Компонент викликає `frame(now, animated)` з обмеженою частотою. */
export class BackdropScene {
  private ctx: CanvasRenderingContext2D;
  private w = 0;
  private h = 0;
  private colors: BackdropColors;
  private stroke: string[] = [];
  private px = new Float32Array(ROWS * COLS);
  private py = new Float32Array(ROWS * COLS);
  private ok = new Uint8Array(ROWS * COLS);
  private hs = new Float32Array(ROWS * COLS);
  private t0 = -1;

  constructor(ctx: CanvasRenderingContext2D, colors: BackdropColors) {
    this.ctx = ctx;
    this.colors = colors;
    this.makeStrokes();
  }

  /** Кольори з токенів (при зміні палітри). */
  setColors(c: BackdropColors): void {
    this.colors = c;
    this.makeStrokes();
  }

  /** Розмір у CSS-пікселях. */
  layout(w: number, h: number): void {
    this.w = w;
    this.h = h;
  }

  /** Рядок `v` (0 — ближній, 1 — дальній): колір від акценту до холодного й прозорість. */
  private makeStrokes(): void {
    const parse = (s: string) => s.split(",").map((n) => Number(n) || 0);
    const a = parse(this.colors.near);
    const b = parse(this.colors.far);
    this.stroke = Array.from({ length: ROWS }, (_, r) => {
      const v = r / (ROWS - 1);
      const m = smooth(0.15, 0.85, v);
      const ch = (i: number) => Math.round((a[i] ?? 0) + ((b[i] ?? 0) - (a[i] ?? 0)) * m);
      // Дальній край згасає до нуля — без різкої лінії горизонту.
      const alpha = (ALPHA_NEAR + (ALPHA_FAR - ALPHA_NEAR) * Math.pow(v, 0.7)) * (1 - smooth(0.72, 1, v));
      return `rgba(${ch(0)},${ch(1)},${ch(2)},${alpha.toFixed(4)})`;
    });
  }

  /** Один кадр. `animated = false` — статичний кадр (зменшений рух). */
  frame(now: number, animated: boolean): void {
    if (this.t0 < 0) this.t0 = now;
    const t = animated ? (now - this.t0) / 1000 : 0;
    const { ctx, w, h } = this;
    ctx.clearRect(0, 0, w, h);
    if (w <= 0 || h <= 0) return;
    // Час поверхні зсунутий: перший кадр — уже «розвинена» складка, а не початкова фаза.
    const ts = t + 30;
    const yaw = YAW + YAW_SWAY * Math.sin(ts * 0.021);
    const cy = Math.cos(yaw);
    const sy = Math.sin(yaw);
    const focal = h * 0.95;
    const horizon = h * 0.08;
    const cx = w * 0.5;
    const { px, py, ok, hs } = this;
    for (let r = 0; r < ROWS; r++) {
      const zl = (r / (ROWS - 1) - 0.5) * DEPTH;
      for (let c = 0; c < COLS; c++) {
        const xl = (c / (COLS - 1) - 0.5) * WIDTH;
        const z = xl * sy + zl * cy + DIST;
        const k = r * COLS + c;
        if (z < NEAR_Z) {
          ok[k] = 0;
          continue;
        }
        const x = xl * cy - zl * sy;
        const y = height(xl, zl, ts);
        hs[k] = y;
        px[k] = cx + (x * focal) / z;
        py[k] = horizon + ((CAM_H - y) * focal) / z;
        ok[k] = 1;
      }
    }
    // Поява за 1.2 с.
    const reveal = animated ? smooth(0, 1.2, t) : 1;
    ctx.lineWidth = 1;
    // Від дальніх до ближніх: ближчі лягають зверху. Рядок малюється шматками: у кожного своя
    // яскравість від нахилу поверхні — м'який полиск там, де складка повернута до світла.
    for (let r = ROWS - 1; r >= 0; r--) {
      ctx.strokeStyle = this.stroke[r] ?? "transparent";
      const next = r < ROWS - 1 ? COLS : -COLS;
      const sign = r < ROWS - 1 ? 1 : -1;
      for (let c0 = 0; c0 < COLS - 1; c0 += CHUNK) {
        const c1 = Math.min(COLS - 1, c0 + CHUNK);
        const km = r * COLS + ((c0 + c1) >> 1);
        if (!ok[r * COLS + c0] || !ok[r * COLS + c1]) continue;
        const slope = (((hs[km + next] ?? 0) - (hs[km] ?? 0)) * sign) / Z_STEP;
        const side = ((hs[km + 1] ?? 0) - (hs[km - 1] ?? 0)) / (2 * X_STEP);
        const lit = clamp(0.5 + 0.55 * slope - 0.35 * side, 0, 1);
        ctx.globalAlpha = reveal * (SHADE_MIN + SHADE_GAIN * lit * lit * lit);
        ctx.beginPath();
        ctx.moveTo(px[r * COLS + c0] ?? 0, py[r * COLS + c0] ?? 0);
        for (let c = c0 + 1; c <= c1; c++) ctx.lineTo(px[r * COLS + c] ?? 0, py[r * COLS + c] ?? 0);
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;
  }
}

/** `#RRGGBB` → `r,g,b` (для кольорів з токенів). */
export function hexToTriplet(hex: string, fallback: string): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m || !m[1]) return fallback;
  const v = parseInt(m[1], 16);
  return `${(v >> 16) & 255},${(v >> 8) & 255},${v & 255}`;
}
