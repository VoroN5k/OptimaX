// Заставка запуску «Pro Premium» (CLAUDE.md 10.5, docs/DESIGN.md розд. 8–9): доріжки потоків
// проявляються зліва направо, курсор шампань проходить ними, з'являється знак і назва; далі знак
// летить на своє місце у верхній панелі, заставка розчиняється, контент з'являється.
//
// ≈2 с, лише transform/opacity (Web Animations API, без canvas і без кадрового циклу в JS).
// Запуск не затримується: заставка грає паралельно із завантаженням даних бекенду. Відповів пізніше —
// знак чекає (до 10 с); раніше — заставка догравається. Клік або клавіша пропускають її.
// Не показується: prefers-reduced-motion, приховане вікно, «Ефекти» не «Повні» (з localStorage,
// бо налаштування ще не прочитані).

const KEY = "optimax.splash";
const LANES = 7;
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

  constructor() {
    document.documentElement.classList.add("splashing");
    // Фон перехоплює кліки, щоб вони не влучили в ще невидимий інтерфейс під ним.
    this.root.setAttribute("aria-hidden", "true");
    this.root.style.cssText =
      "position:fixed;inset:0;z-index:1000;overflow:hidden;cursor:default;background:var(--bg);";
    const field = el(
      "div",
      "position:absolute;left:12%;right:12%;top:50%;height:" +
        `${LANES * 26}px;transform:translateY(-50%);` +
        "-webkit-mask-image:radial-gradient(ellipse 60% 80% at 50% 50%,#000 40%,transparent);" +
        "mask-image:radial-gradient(ellipse 60% 80% at 50% 50%,#000 40%,transparent);",
      this.root,
    );
    for (let i = 0; i < LANES; i++) {
      const lane = el(
        "div",
        `position:absolute;left:0;right:0;top:${i * 26}px;height:6px;border-radius:2px;transform-origin:left center;` +
          // Сегменти «працював / спав» — декоративний ритм, без даних і без підписів.
          "background:repeating-linear-gradient(90deg," +
          `color-mix(in srgb,var(--thread-work) 32%,transparent) 0 ${60 + ((i * 37) % 90)}px,transparent 0 ${78 + ((i * 37) % 90)}px,` +
          `color-mix(in srgb,var(--thread-idle) 14%,transparent) 0 ${120 + ((i * 53) % 70)}px,transparent 0 ${128 + ((i * 53) % 70)}px);` +
          `background-position:${-(i * 71) % 160}px 0;`,
        field,
      );
      lane.animate(
        [
          { transform: "scaleX(0)", opacity: 0 },
          { transform: "scaleX(1)", opacity: 1 },
        ],
        { duration: 520, delay: 40 * i, easing: EASE, fill: "backwards" },
      );
    }
    const cursor = el(
      "div",
      "position:absolute;top:-24px;bottom:-24px;left:0;width:100px;" +
        "background:linear-gradient(90deg,rgba(var(--glow),0),rgba(var(--glow),.12));" +
        "box-shadow:1px 0 0 rgba(var(--glow),.5);opacity:0;",
      field,
    );
    cursor.animate(
      [
        { transform: "translateX(-120px)", opacity: 0 },
        { opacity: 1, offset: 0.15 },
        { opacity: 1, offset: 0.85 },
        { transform: `translateX(${Math.round(window.innerWidth * 0.76) + 20}px)`, opacity: 0 },
      ],
      { duration: 1100, delay: 250, easing: "cubic-bezier(.45,0,.55,1)", fill: "both" },
    );
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
    window.removeEventListener("pointerdown", this.skip, true);
    window.removeEventListener("keydown", this.skip, true);
    this.root.remove();
    document.documentElement.classList.remove("splashing");
  }
}
