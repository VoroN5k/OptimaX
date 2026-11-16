// Фоновий шар «Шовк» (docs/DESIGN.md розд. 9, CLAUDE.md 10.4) і заставка запуску, що складає ту
// саму поверхню з точок (`assemble`). Без залежностей.
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

/** Момент поверхні, з якого починається фон і яким закінчується заставка, с. */
const SURFACE_T0 = 30;
/** Заставка малює кожну другу точку рядка — образ той самий, точок удвічі менше. */
const DOT_STEP = 2;

const clamp = (v: number, a: number, b: number) => (v < a ? a : v > b ? b : v);
const easeOut = (k: number) => 1 - Math.pow(1 - clamp(k, 0, 1), 3);
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
  private skipReveal = false;
  private dot: string[] = [];
  // Заставка: звідки летить кожна точка і з якою затримкою (частка шляху 0–0.2).
  private fromX = new Float32Array(ROWS * COLS);
  private fromY = new Float32Array(ROWS * COLS);
  private delay = new Float32Array(ROWS * COLS);
  private fromW = -1;
  private fromH = -1;

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

  /** Рядок `v` (0 — ближній, 1 — дальній): колір від акценту до холодного й прозорість. Точки
   *  заставки — того самого кольору, але яскравіші (на порожньому екрані вони — головний образ). */
  private makeStrokes(): void {
    const parse = (s: string) => s.split(",").map((n) => Number(n) || 0);
    const a = parse(this.colors.near);
    const b = parse(this.colors.far);
    const rows = Array.from({ length: ROWS }, (_, r) => {
      const v = r / (ROWS - 1);
      const m = smooth(0.15, 0.85, v);
      const ch = (i: number) => Math.round((a[i] ?? 0) + ((b[i] ?? 0) - (a[i] ?? 0)) * m);
      // Дальній край згасає до нуля — без різкої лінії горизонту.
      const alpha = (ALPHA_NEAR + (ALPHA_FAR - ALPHA_NEAR) * Math.pow(v, 0.7)) * (1 - smooth(0.72, 1, v));
      return { rgb: `${ch(0)},${ch(1)},${ch(2)}`, alpha };
    });
    this.stroke = rows.map((r) => `rgba(${r.rgb},${r.alpha.toFixed(4)})`);
    this.dot = rows.map((r) => `rgba(${r.rgb},${Math.min(0.9, r.alpha * 4).toFixed(4)})`);
  }

  /** Проєкція сітки поверхні на екран у момент `ts` (с) — у типізовані масиви, без алокацій. */
  private project(ts: number): void {
    const { w, h, px, py, ok, hs } = this;
    const yaw = YAW + YAW_SWAY * Math.sin(ts * 0.021);
    const cy = Math.cos(yaw);
    const sy = Math.sin(yaw);
    const focal = h * 0.95;
    const horizon = h * 0.08;
    const cx = w * 0.5;
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
  }

  /** Лінії рядків з полиском; `alpha` — загальна прозорість (поява). */
  private drawLines(alpha: number): void {
    const { ctx, px, py, ok, hs } = this;
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
        ctx.globalAlpha = alpha * (SHADE_MIN + SHADE_GAIN * lit * lit * lit);
        ctx.beginPath();
        ctx.moveTo(px[r * COLS + c0] ?? 0, py[r * COLS + c0] ?? 0);
        for (let c = c0 + 1; c <= c1; c++) ctx.lineTo(px[r * COLS + c] ?? 0, py[r * COLS + c] ?? 0);
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;
  }

  /** Один кадр. `animated = false` — статичний кадр (зменшений рух). */
  frame(now: number, animated: boolean): void {
    if (this.t0 < 0) this.t0 = now;
    const t = animated ? (now - this.t0) / 1000 : 0;
    const { ctx, w, h } = this;
    ctx.clearRect(0, 0, w, h);
    if (w <= 0 || h <= 0) return;
    // Час поверхні зсунутий: перший кадр — уже «розвинена» складка, та сама, якою закінчується
    // заставка (`assemble`), тож перехід від заставки до фону безшовний.
    this.project(t + SURFACE_T0);
    // Поява за 1.2 с (після заставки — одразу: поверхня вже на екрані).
    this.drawLines(animated && !this.skipReveal ? smooth(0, 1.2, t) : 1);
  }

  /** Заставка щойно показала ту саму поверхню: фон стартує з її кадру, одразу, без проявлення. */
  continueSplash(): void {
    this.skipReveal = true;
    this.t0 = -1;
  }

  /**
   * Кадр заставки: точки сітки злітаються з розсипу у поверхню (`k` 0→1), наприкінці між ними
   * проступають лінії. Поверхня — та сама, що в першому кадрі фону.
   */
  assemble(k: number): void {
    const { ctx, w, h, px, py, ok } = this;
    ctx.clearRect(0, 0, w, h);
    if (w <= 0 || h <= 0) return;
    if (this.fromW !== w || this.fromH !== h) this.scatter();
    this.project(SURFACE_T0);
    const lines = smooth(0.62, 1, k);
    if (lines > 0) this.drawLines(lines);
    const dotFade = 1 - 0.75 * lines;
    for (let r = ROWS - 1; r >= 0; r--) {
      ctx.fillStyle = this.dot[r] ?? "transparent";
      for (let c = 0; c < COLS; c += DOT_STEP) {
        const i = r * COLS + c;
        if (!ok[i]) continue;
        // Кожна точка вилітає зі своєю затримкою; дорога — з плавним гальмуванням.
        const d = this.delay[i] ?? 0;
        const p = easeOut((k - d) / (0.7 - 0.2));
        if (p <= 0) continue;
        const fx = this.fromX[i] ?? 0;
        const fy = this.fromY[i] ?? 0;
        const x = fx + ((px[i] ?? 0) - fx) * p;
        const y = fy + ((py[i] ?? 0) - fy) * p;
        ctx.globalAlpha = dotFade * Math.min(1, p * 1.6);
        ctx.fillRect(x - 0.8, y - 0.8, 1.6, 1.6);
      }
    }
    ctx.globalAlpha = 1;
  }

  /** Початкові положення точок заставки: детермінований розсип по всьому вікну. */
  private scatter(): void {
    this.fromW = this.w;
    this.fromH = this.h;
    let s = 0x2f6b9d;
    const rnd = () => {
      s = (s + 0x6d2b79f5) | 0;
      let t = Math.imul(s ^ (s >>> 15), 1 | s);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    for (let i = 0; i < ROWS * COLS; i++) {
      this.fromX[i] = (rnd() * 1.4 - 0.2) * this.w;
      this.fromY[i] = (rnd() * 1.4 - 0.2) * this.h;
      this.delay[i] = rnd() * 0.2;
    }
  }
}

/** Кольори поверхні з токенів (`--glow` — ближні рядки, `--thread-idle` — дальні). */
export function backdropColors(): BackdropColors {
  const css = getComputedStyle(document.documentElement);
  return {
    near: css.getPropertyValue("--glow").replace(/\s/g, "") || "233,205,140",
    far: hexToTriplet(css.getPropertyValue("--thread-idle"), "143,148,170"),
  };
}

/** `#RRGGBB` → `r,g,b` (для кольорів з токенів). */
export function hexToTriplet(hex: string, fallback: string): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m || !m[1]) return fallback;
  const v = parseInt(m[1], 16);
  return `${(v >> 16) & 255},${(v >> 8) & 255},${v & 255}`;
}
