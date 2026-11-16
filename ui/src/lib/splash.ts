// Заставка запуску «Pro Premium» (CLAUDE.md 10.5, docs/DESIGN.md розд. 8–9): точки злітаються
// з розсипу й складаються в поверхню фону «Шовк», між ними проступають лінії; з'являється знак і
// назва; далі знак летить на своє місце у верхній панелі, заставка розчиняється, а фон продовжує
// ту саму поверхню.
//
// ≈2 с: canvas 2D (кадровий цикл лише на час складання, DPR ≤ 2) + Web Animations для знака.
// Запуск не затримується: заставка грає паралельно із завантаженням даних бекенду. Відповів пізніше —
// знак чекає (до 10 с); раніше — заставка догравається. Клік або клавіша пропускають її.
// Не показується: prefers-reduced-motion, приховане вікно, «Ефекти» не «Повні» (з localStorage,
// бо налаштування ще не прочитані).

import { BackdropScene, backdropColors } from "./backdrop";

const KEY = "optimax.splash";
/** Скільки триває складання поверхні з точок. */
const ASSEMBLE_MS = 1400;
const EASE = "cubic-bezier(.2,.8,.2,1)";
const HOLD_UNTIL_MS = 1700; // мінімальна тривалість до виходу
const OUT_MS = 520;
const SKIP_MS = 200;
const MAX_WAIT_MS = 10_000;
const MARK_PX = 56;

/** Керування заставкою з `main.ts`. */
export interface Splash {
  /** Інтерфейс змонтовано: можна летіти до знака у верхній панелі. */
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

/** Стартує заставку або повертає `null`, якщо її не треба показувати. */
export function startSplash(): Splash | null {
  if (!allowed()) return null;
  const run = new SplashRun();
  return { ready: () => run.ready(), cancel: () => run.cancel() };
}

function el(tag: string, css: string, parent: HTMLElement): HTMLElement {
  const e = document.createElement(tag);
  e.style.cssText = css;
  parent.append(e);
  return e;
}

class SplashRun {
  private root = document.createElement("div");
  private mark: HTMLElement;
  private name: HTMLElement;
  private t0 = performance.now();
  private isReady = false;
  private skipped = false;
  private out = false;
  private done = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private scene: BackdropScene | null = null;
  private raf = 0;

  constructor() {
    document.documentElement.classList.add("splashing");
    // Фон перехоплює кліки, щоб вони не влучили в ще невидимий інтерфейс під ним.
    this.root.setAttribute("aria-hidden", "true");
    this.root.style.cssText =
      "position:fixed;inset:0;z-index:1000;overflow:hidden;cursor:default;background:var(--bg);";
    // Поверхня «Шовк» складається з точок, що злітаються з розсипу; наприкінці проступають лінії.
    // Це той самий перший кадр, з якого стартує фон, тож після заставки фон продовжує її без стрибка.
    const canvas = el("canvas", "position:absolute;inset:0;width:100%;height:100%;display:block;" +
        // Та сама маска бічних країв, що й у фону (Backdrop.svelte), — перехід без різниці.
        "-webkit-mask-image:linear-gradient(90deg,transparent,#000 18%,#000 82%,transparent);" +
        "mask-image:linear-gradient(90deg,transparent,#000 18%,#000 82%,transparent);", this.root) as HTMLCanvasElement;
    const ctx = canvas.getContext("2d", { alpha: true });
    if (ctx) {
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      const w = window.innerWidth;
      const h = window.innerHeight;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      this.scene = new BackdropScene(ctx, backdropColors());
      this.scene.layout(w, h);
      this.raf = requestAnimationFrame(this.draw);
    }
    const center =
      "position:absolute;left:50%;top:50%;display:flex;flex-direction:column;align-items:center;gap:16px;transform:translate(-50%,-50%);";
    const box = el("div", center, this.root);
    this.mark = el(
      "span",
      `display:block;width:${MARK_PX}px;height:${MARK_PX}px;border-radius:16px;` +
        "background:var(--accent-gradient);box-shadow:0 0 40px rgba(var(--glow),.55),0 0 0 1px rgba(255,255,255,.12) inset;",
      box,
    );
    this.mark.animate(
      [
        { transform: "scale(.6)", opacity: 0 },
        { transform: "scale(1)", opacity: 1 },
      ],
      { duration: 480, delay: 650, easing: EASE, fill: "backwards" },
    );
    this.name = el(
      "span",
      "font:300 28px/1 var(--font-ui, 'Inter Tight', sans-serif);letter-spacing:.04em;color:var(--text);",
      box,
    );
    this.name.textContent = "OptimaX";
    this.name.animate(
      [
        { transform: "translateY(8px)", opacity: 0 },
        { transform: "none", opacity: 1 },
      ],
      { duration: 480, delay: 820, easing: EASE, fill: "backwards" },
    );
    document.body.append(this.root);
    window.addEventListener("pointerdown", this.skip, true);
    window.addEventListener("keydown", this.skip, true);
    this.timer = setTimeout(this.tick, HOLD_UNTIL_MS);
  }

  /** Кадр складання: прогрес за часом; після кінця цикл зупиняється (останній кадр лишається). */
  private draw = (now: number): void => {
    const k = Math.min(1, (now - this.t0) / ASSEMBLE_MS);
    this.scene?.assemble(k);
    this.raf = k < 1 && !this.done ? requestAnimationFrame(this.draw) : 0;
  };

  ready(): void {
    this.isReady = true;
    this.tick();
  }

  cancel(): void {
    this.isReady = true;
    this.skipped = true;
    this.finish();
  }

  private skip = (): void => {
    this.skipped = true;
    this.tick();
  };

  private tick = (): void => {
    if (this.out || this.done) return;
    const t = performance.now() - this.t0;
    if (this.isReady && (this.skipped || t >= HOLD_UNTIL_MS)) this.leave();
    else if (t >= MAX_WAIT_MS) this.finish();
    else {
      clearTimeout(this.timer);
      this.timer = setTimeout(this.tick, Math.max(50, HOLD_UNTIL_MS - t));
    }
  };

  /** Вихід: знак летить до знака у верхній панелі, фон розчиняється, контент з'являється. */
  private leave(): void {
    this.out = true;
    clearTimeout(this.timer);
    const ms = this.skipped ? SKIP_MS : OUT_MS;
    this.root.style.pointerEvents = "none";
    const target = document.querySelector<HTMLElement>("[data-splash-mark]")?.getBoundingClientRect();
    const from = this.mark.getBoundingClientRect();
    if (target && target.width > 0 && from.width > 0) {
      const dx = target.left + target.width / 2 - (from.left + from.width / 2);
      const dy = target.top + target.height / 2 - (from.top + from.height / 2);
      this.mark.animate([{ transform: "none" }, { transform: `translate(${dx}px,${dy}px) scale(${target.width / from.width})` }], {
        duration: ms,
        easing: EASE,
        fill: "forwards",
      });
    }
    this.name.animate([{ opacity: 1 }, { opacity: 0 }], { duration: ms * 0.5, fill: "forwards" });
    // Контент з'являється разом із розчиненням (transition у App.svelte).
    document.documentElement.classList.remove("splashing");
    const fade = this.root.animate([{ opacity: 1 }, { opacity: 0 }], {
      duration: ms,
      delay: ms * 0.35,
      easing: EASE,
      fill: "forwards",
    });
    fade.onfinish = () => this.finish();
  }

  private finish(): void {
    if (this.done) return;
    this.done = true;
    clearTimeout(this.timer);
    cancelAnimationFrame(this.raf);
    window.removeEventListener("pointerdown", this.skip, true);
    window.removeEventListener("keydown", this.skip, true);
    this.root.remove();
    document.documentElement.classList.remove("splashing");
  }
}
