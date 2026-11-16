<script lang="ts">
  import type { FrametimeChart } from "../lib/bindings/FrametimeChart";
  import type { ThreadLane } from "../lib/bindings/ThreadLane";
  import type { TimelineView } from "../lib/bindings/TimelineView";
  import { t, tk } from "../lib/i18n";

  // Шкала запису: кадри, частини кадру, черга до GPU, потоки гри, завантаження й драйвери на
  // одній осі часу, з масштабом і переходом між ривками. Дані й ривки готує бекенд; тут лише
  // показ. Canvas перемальовується тільки на зміну стану (без циклу анімації).
  let { view, chart = null }: { view: TimelineView; chart?: FrametimeChart | null } = $props();

  const LABEL_W = 104;
  const hintId = `tl-hint-${Math.random().toString(36).slice(2, 9)}`;
  const LANE_GAP = 6;
  const AXIS_H = 18;
  /** Найменший видимий відрізок, мс. */
  const MIN_SPAN = 20;
  /** Наближення, з якого показуються точні стани потоків (якщо вони є в цьому місці), мс. */
  const DETAIL_SPAN = 3000;

  type LaneId = "frames" | "work" | "queue" | "main" | "render" | "load" | "drivers";
  type Lane = { id: LaneId; h: number; label: string; y: number };

  const hasQueue = $derived(view.frames.queue.some((v) => v != null));
  const hasWork = $derived(view.frames.cpu.some((v) => v != null) || view.frames.gpu.some((v) => v != null));
  const lanes = $derived.by(() => {
    const out: Lane[] = [];
    let y = 0;
    const add = (id: LaneId, h: number, label: string) => {
      out.push({ id, h, label, y });
      y += h + LANE_GAP;
    };
    add("frames", 84, tk("ui.tl.lane.frames"));
    if (hasWork) add("work", 56, tk("ui.tl.lane.work"));
    if (hasQueue) add("queue", 36, tk("ui.tl.lane.queue"));
    if (view.main) add("main", 22, tk("ui.tl.lane.main"));
    if (view.render) add("render", 22, tk("ui.tl.lane.render"));
    if (view.samples.t.length) add("load", 40, tk("ui.tl.lane.load"));
    if (view.dpc.length || view.shader_writes.length) add("drivers", 24, tk("ui.tl.lane.drivers"));
    return out;
  });
  const plotH = $derived(lanes.reduce((s, l) => s + l.h + LANE_GAP, 0));
  const height = $derived(plotH + AXIS_H);

  // ---- Видимий відрізок ----
  const fullFrom = $derived(view.start_ms);
  const fullTo = $derived(Math.max(view.end_ms, view.start_ms + MIN_SPAN));
  let from = $state(0);
  let to = $state(1);
  $effect(() => {
    from = fullFrom;
    to = fullTo;
  });

  function setRange(a: number, b: number) {
    const span = Math.min(Math.max(b - a, MIN_SPAN), fullTo - fullFrom);
    const lo = Math.max(fullFrom, Math.min(a, fullTo - span));
    from = lo;
    to = lo + span;
  }
  function zoom(factor: number, at = (from + to) / 2) {
    const k = (at - from) / (to - from);
    const span = (to - from) * factor;
    setRange(at - span * k, at - span * k + span);
  }
  function pan(ms: number) {
    setRange(from + ms, to + ms);
  }

  // ---- Ривки ----
  let selected = $state(-1);
  const current = $derived(view.spikes[selected] ?? null);
  /** Причини з радара для обраного ривку (радар позначає лише найдовші). */
  const causes = $derived.by(() => {
    if (!current || !chart) return [];
    const mark = chart.spikes.find((s) => Math.abs(s.t_ms - current[0]) < 0.5);
    return mark ? mark.causes.map((c) => t(c)) : [];
  });
  function jump(i: number) {
    const s = view.spikes[i];
    if (!s) return;
    selected = i;
    const [at, ft] = s;
    setRange(at - ft - 400, at + 300);
  }
  function worst() {
    let best = -1;
    let longest = -1;
    view.spikes.forEach(([, ft], i) => {
      if (ft > longest) {
        longest = ft;
        best = i;
      }
    });
    jump(best);
  }
  /** Наступний/попередній ривок відносно обраного або центру видимого відрізка. */
  function step(dir: 1 | -1) {
    if (!view.spikes.length) return;
    if (selected >= 0) {
      jump(Math.max(0, Math.min(view.spikes.length - 1, selected + dir)));
      return;
    }
    const mid = (from + to) / 2;
    const i = lowerBound(view.spikes.length, (k) => (view.spikes[k]?.[0] ?? 0) < mid);
    jump(dir > 0 ? Math.min(i, view.spikes.length - 1) : Math.max(i - 1, 0));
  }

  /** Перший індекс у [0, n), для якого `before` хибне (масив упорядкований). */
  function lowerBound(n: number, before: (i: number) => boolean): number {
    let lo = 0;
    let hi = n;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      if (before(m)) lo = m + 1;
      else hi = m;
    }
    return lo;
  }

  /** Кадри-ривки (для кольору стовпців). */
  const spikeFrame = $derived.by(() => {
    const flags = new Uint8Array(view.frames.t.length);
    for (const [at] of view.spikes) {
      const i = lowerBound(view.frames.t.length, (k) => (view.frames.t[k] ?? 0) < at - 0.01);
      if (i < flags.length) flags[i] = 1;
    }
    return flags;
  });

  // ---- Полотно ----
  let wrap: HTMLDivElement | undefined = $state();
  let canvas: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let hover = $state<number | null>(null);
  const plotW = $derived(Math.max(width - LABEL_W, 1));
  const x = (ms: number) => LABEL_W + ((ms - from) / (to - from)) * plotW;
  const tAt = (px: number) => from + ((px - LABEL_W) / plotW) * (to - from);

  $effect(() => {
    if (!wrap) return;
    const ro = new ResizeObserver(([e]) => {
      if (e) width = Math.floor(e.contentRect.width);
    });
    ro.observe(wrap);
    return () => ro.disconnect();
  });

  // Кожна метрика — свій колір зі змістом (CLAUDE.md 10.2, docs/DESIGN.md розд. 4): кадри — шампань,
  // CPU — світла шампань, GPU — блакитний, черга — фіолетовий, ядро — блакитний, завантаження GPU —
  // зелений, ривок — помаранчевий, драйвери й диск — червоний; стани потоку — індиго / помаранчевий /
  // лавандовий / червоний / темний. Однаковий колір для різних ліній робив шкалу нечитабельною.
  type Colors = Record<
    | "text"
    | "muted"
    | "dim"
    | "border"
    | "input"
    | "frame"
    | "cpu"
    | "gpu"
    | "queue"
    | "core"
    | "gpuLoad"
    | "warn"
    | "danger"
    | "run"
    | "sleep",
    string
  >;
  let colors: Colors | null = null;
  function readColors(el: HTMLElement): Colors {
    const cs = getComputedStyle(el);
    const v = (n: string) => cs.getPropertyValue(n).trim();
    return {
      text: v("--text"),
      muted: v("--text-muted"),
      dim: v("--text-dim"),
      border: v("--border"),
      input: v("--bg-input"),
      frame: v("--a2"),
      cpu: v("--a1"),
      gpu: v("--gpu"),
      queue: v("--queue"),
      core: v("--gpu"),
      gpuLoad: v("--ok"),
      warn: v("--warn"),
      danger: v("--bad"),
      run: v("--thread-work"),
      sleep: v("--thread-sleep"),
    };
  }
  /** Кольори станів потоку в порядку `slot`: працює / ядро / синхр. / диск / сон / інше. */
  const stateColor = (c: Colors) => [c.run, c.warn, c.queue, c.danger, c.sleep, c.dim];

  let frameReq = 0;
  $effect(() => {
    // Залежності перемальовування.
    void [from, to, width, hover, selected, lanes, view];
    if (!canvas) return;
    cancelAnimationFrame(frameReq);
    frameReq = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frameReq);
  });

  /** Точки лінії для серії кадрів: стовпцями (максимум у пікселі) або по кадрах. */
  function framePoints(vals: (number | null)[], lo: number, hi: number, perPx: boolean): [number, number, number][] {
    const tt = view.frames.t;
    const pts: [number, number, number][] = [];
    if (!perPx) {
      for (let i = lo; i < hi; i++) {
        const v = vals[i];
        const at = tt[i];
        const len = view.frames.ft[i];
        if (v == null || at == null || len == null) continue;
        // Значення стосується всього кадру — сходинкою від початку до Present.
        pts.push([x(at - len), v, at - len], [x(at), v, at]);
      }
      return pts;
    }
    let col = -1;
    let best = -1;
    let bestT = 0;
    for (let i = lo; i < hi; i++) {
      const v = vals[i];
      const at = tt[i];
      if (v == null || at == null) continue;
      const c = Math.floor(x(at));
      if (c !== col) {
        if (col >= 0) pts.push([col + 0.5, best, bestT]);
        col = c;
        best = v;
        bestT = at;
      } else if (v > best) best = v;
    }
    if (col >= 0) pts.push([col + 0.5, best, bestT]);
    return pts;
  }

  /** Чи лежить між двома моментами розрив запису (тоді лінію не з'єднуємо). */
  function crossesGap(a: number, b: number): boolean {
    return view.gaps.some(([g0, g1]) => g0 >= a - 0.01 && g1 <= b + 0.01);
  }

  function line(
    ctx: CanvasRenderingContext2D,
    lane: Lane,
    pts: [number, number, number][],
    y: (v: number) => number,
    color: string,
  ) {
    ctx.save();
    ctx.beginPath();
    ctx.rect(LABEL_W, lane.y, plotW, lane.h);
    ctx.clip();
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.25;
    ctx.beginPath();
    let prevT: number | null = null;
    for (const [px, v, at] of pts) {
      if (prevT == null || crossesGap(prevT, at)) ctx.moveTo(px, y(v));
      else ctx.lineTo(px, y(v));
      prevT = at;
    }
    ctx.stroke();
    ctx.restore();
  }

  function drawThread(ctx: CanvasRenderingContext2D, c: Colors, lane: Lane, th: ThreadLane) {
    const cols = stateColor(c);
    const top = lane.y;
    const h = lane.h;
    const b0 = Math.max(0, Math.floor((from - th.start_ms) / th.bucket_ms));
    const b1 = Math.min(th.buckets.length, Math.ceil((to - th.start_ms) / th.bucket_ms) + 1);
    const det = to - from <= DETAIL_SPAN ? th.detail : [];
    const d0 = lowerBound(det.length, (k) => {
      const d = det[k];
      return d ? d[0] + d[1] < from : false;
    });
    const showDetail = d0 < det.length && (det[d0]?.[0] ?? Infinity) < to;
    ctx.globalAlpha = showDetail ? 0.3 : 0.9;
    for (let b = b0; b < b1; b++) {
      const bucket = th.buckets[b];
      if (!bucket) continue;
      const a = th.start_ms + b * th.bucket_ms;
      const x0 = Math.max(x(a), LABEL_W);
      const x1 = Math.min(x(a + th.bucket_ms), LABEL_W + plotW);
      if (x1 <= x0) continue;
      let yy = top + h;
      bucket.forEach((ms, s) => {
        const part = (Math.min(ms, th.bucket_ms) / th.bucket_ms) * h;
        if (part <= 0) return;
        ctx.fillStyle = cols[s] ?? c.dim;
        ctx.fillRect(x0, yy - part, Math.max(x1 - x0 - 0.5, 0.5), part);
        yy -= part;
      });
    }
    ctx.globalAlpha = 1;
    if (!showDetail) return;
    // Проміжки бувають коротші за піксель (тисячі перемикань за секунду): у кожному пікселі
    // малюємо стан, що забрав найбільше часу; піксель без даних хоча б на половину — пропускаємо.
    const pxMs = (to - from) / plotW;
    const cols0 = plotW + 1;
    const acc = new Float32Array(cols0 * 6);
    for (let k = d0; k < det.length; k++) {
      const d = det[k];
      if (!d || d[0] >= to) break;
      const a = Math.max(d[0], from);
      const b = Math.min(d[0] + d[1], to);
      const st = Math.min(d[2], 5);
      for (let col = Math.floor((a - from) / pxMs); col < cols0; col++) {
        const c0 = from + col * pxMs;
        const part = Math.min(b, c0 + pxMs) - Math.max(a, c0);
        if (part <= 0) break;
        acc[col * 6 + st] = (acc[col * 6 + st] ?? 0) + part;
      }
    }
    for (let col = 0; col < cols0; col++) {
      let best = 0;
      let total = 0;
      for (let st = 0; st < 6; st++) {
        const v = acc[col * 6 + st] ?? 0;
        total += v;
        if (v > (acc[col * 6 + best] ?? 0)) best = st;
      }
      if (total < pxMs * 0.5) continue;
      ctx.fillStyle = cols[best] ?? c.dim;
      ctx.fillRect(LABEL_W + col, top, 1, h);
    }
  }

  function niceStep(span: number): number {
    const raw = (span / plotW) * 90;
    const p = 10 ** Math.floor(Math.log10(raw));
    for (const m of [1, 2, 5, 10]) if (p * m >= raw) return p * m;
    return p * 10;
  }

  function draw() {
    if (!canvas || width <= 0) return;
    colors ??= readColors(canvas);
    const c = colors;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.font = "11px Inter, system-ui, sans-serif";
    ctx.textBaseline = "middle";

    const tt = view.frames.t;
    const ft = view.frames.ft;
    // Кадри, що хоч частково у видимому відрізку.
    const lo = Math.max(0, lowerBound(tt.length, (k) => (tt[k] ?? 0) < from) - 1);
    const hi = Math.min(tt.length, lowerBound(tt.length, (k) => (tt[k] ?? 0) <= to) + 1);
    const perPx = hi - lo > plotW;
    let visMax = 0;
    for (let i = lo; i < hi; i++) visMax = Math.max(visMax, ft[i] ?? 0);
    const med = view.median_ms || 1;
    const span = to - from;
    const yMax =
      span > 5000 ? Math.max(med * 2, Math.min(visMax, Math.max(med * 12, 50))) : Math.max(med * 2, visMax * 1.05);

    for (const lane of lanes) {
      ctx.fillStyle = c.input;
      ctx.fillRect(LABEL_W, lane.y, plotW, lane.h);
      // Доріжки зі шкалою в мс: межа шкали — під назвою, щоб не перекривати лінії.
      const scale = lane.id === "frames" || lane.id === "work" || lane.id === "queue";
      ctx.fillStyle = c.muted;
      ctx.fillText(lane.label, 0, lane.y + lane.h / 2 - (scale ? 7 : 0), LABEL_W - 8);
    }
    const scaleLabel = (lane: Lane, v: number) => {
      ctx.fillStyle = c.dim;
      ctx.fillText(tk("ui.tl.scale", { v: v.toFixed(0) }), 0, lane.y + lane.h / 2 + 7, LABEL_W - 8);
    };

    // Розриви (не зараховано).
    ctx.fillStyle = c.border;
    for (const [a, b] of view.gaps) {
      if (b < from || a > to) continue;
      const x0 = Math.max(x(a), LABEL_W);
      const x1 = Math.min(x(b), LABEL_W + plotW);
      ctx.fillRect(x0, 0, x1 - x0, plotH - LANE_GAP);
    }

    // Обраний ривок — смуга через усі доріжки.
    if (current) {
      const x0 = Math.max(x(current[0] - current[1]), LABEL_W);
      const x1 = Math.min(x(current[0]), LABEL_W + plotW);
      if (x1 > LABEL_W && x0 < LABEL_W + plotW) {
        ctx.globalAlpha = 0.18;
        ctx.fillStyle = c.warn;
        ctx.fillRect(x0, 0, Math.max(x1 - x0, 2), plotH - LANE_GAP);
        ctx.globalAlpha = 1;
      }
    }

    for (const lane of lanes) {
      const top = lane.y;
      const h = lane.h;
      const yv = (v: number, max: number) => top + h - (Math.min(v, max) / max) * h;
      if (lane.id === "frames") {
        if (perPx) {
          const colMax = new Float32Array(plotW + 1);
          const colSpike = new Uint8Array(plotW + 1);
          for (let i = lo; i < hi; i++) {
            const at = tt[i];
            if (at == null) continue;
            const col = Math.floor(x(at) - LABEL_W);
            if (col < 0 || col > plotW) continue;
            colMax[col] = Math.max(colMax[col] ?? 0, ft[i] ?? 0);
            if (spikeFrame[i]) colSpike[col] = 1;
          }
          for (let col = 0; col <= plotW; col++) {
            const v = colMax[col] ?? 0;
            if (v <= 0) continue;
            ctx.fillStyle = colSpike[col] ? c.warn : c.frame;
            // Звичайні кадри напівпрозорі — ривок виділяється на їхньому тлі.
            ctx.globalAlpha = colSpike[col] ? 1 : 0.55;
            const y0 = yv(v, yMax);
            ctx.fillRect(LABEL_W + col, y0, 1, top + h - y0);
          }
        } else {
          for (let i = lo; i < hi; i++) {
            const at = tt[i];
            const v = ft[i];
            if (at == null || v == null) continue;
            const x0 = Math.max(x(at - v), LABEL_W);
            const x1 = Math.min(x(at), LABEL_W + plotW);
            if (x1 <= x0) continue;
            ctx.fillStyle = spikeFrame[i] ? c.warn : c.frame;
            ctx.globalAlpha = spikeFrame[i] ? 1 : 0.55;
            const y0 = yv(v, yMax);
            ctx.fillRect(x0, y0, Math.max(x1 - x0 - 1, 1), top + h - y0);
          }
        }
        ctx.globalAlpha = 1;
        // Медіана.
        ctx.strokeStyle = c.muted;
        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(LABEL_W, yv(med, yMax));
        ctx.lineTo(LABEL_W + plotW, yv(med, yMax));
        ctx.stroke();
        ctx.setLineDash([]);
        scaleLabel(lane, yMax);
      } else if (lane.id === "work") {
        scaleLabel(lane, yMax);
        line(ctx, lane, framePoints(view.frames.cpu, lo, hi, perPx), (v) => yv(v, yMax), c.cpu);
        line(ctx, lane, framePoints(view.frames.gpu, lo, hi, perPx), (v) => yv(v, yMax), c.gpu);
      } else if (lane.id === "queue") {
        const pts = framePoints(view.frames.queue, lo, hi, perPx);
        const qMax = Math.max(1, ...pts.map((p) => p[1])) * 1.1;
        line(ctx, lane, pts, (v) => yv(v, qMax), c.queue);
        scaleLabel(lane, qMax);
      } else if (lane.id === "main" && view.main) {
        drawThread(ctx, c, lane, view.main);
      } else if (lane.id === "render" && view.render) {
        drawThread(ctx, c, lane, view.render);
      } else if (lane.id === "load") {
        const st = view.samples.t;
        const s0 = Math.max(0, lowerBound(st.length, (k) => (st[k] ?? 0) < from) - 1);
        const s1 = Math.min(st.length, lowerBound(st.length, (k) => (st[k] ?? 0) <= to) + 1);
        const series = (vals: (number | null)[]): [number, number, number][] => {
          const out: [number, number, number][] = [];
          for (let i = s0; i < s1; i++) {
            const v = vals[i];
            const at = st[i];
            if (v != null && at != null) out.push([x(at), v, at]);
          }
          return out;
        };
        line(ctx, lane, series(view.samples.core), (v) => yv(v, 100), c.core);
        line(ctx, lane, series(view.samples.gpu), (v) => yv(v, 100), c.gpuLoad);
      } else if (lane.id === "drivers") {
        ctx.fillStyle = c.danger;
        for (const d of view.dpc) {
          if (d.t < from || d.t > to) continue;
          const k = Math.min(1, Math.max(0.3, d.us / 2000));
          ctx.fillRect(x(d.t), top + h - k * h, Math.max(x(d.t + d.us / 1000) - x(d.t), 1.5), k * h);
        }
        ctx.fillStyle = c.warn;
        for (const w of view.shader_writes) {
          if (w < from || w > to) continue;
          ctx.beginPath();
          ctx.arc(x(w), top + 5, 2.5, 0, Math.PI * 2);
          ctx.fill();
        }
      }
    }

    // Вісь часу: секунди від початку шкали (як у радарі).
    const stepMs = niceStep(span);
    const digits = stepMs >= 1000 ? 0 : stepMs >= 100 ? 1 : stepMs >= 10 ? 2 : 3;
    ctx.fillStyle = c.dim;
    ctx.strokeStyle = c.border;
    ctx.textAlign = "center";
    for (let at = Math.ceil((from - fullFrom) / stepMs) * stepMs + fullFrom; at <= to; at += stepMs) {
      const px = x(at);
      ctx.beginPath();
      ctx.moveTo(px, plotH - LANE_GAP);
      ctx.lineTo(px, plotH - LANE_GAP + 4);
      ctx.stroke();
      ctx.fillText(`${((at - fullFrom) / 1000).toFixed(digits)} ${tk("ui.tl.s")}`, px, plotH + AXIS_H / 2);
    }
    ctx.textAlign = "left";

    if (hover != null && hover >= from && hover <= to) {
      ctx.strokeStyle = c.text;
      ctx.globalAlpha = 0.5;
      ctx.beginPath();
      ctx.moveTo(x(hover) + 0.5, 0);
      ctx.lineTo(x(hover) + 0.5, plotH - LANE_GAP);
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
  }

  // ---- Миша й клавіатура ----
  let drag: { px: number; from: number; to: number } | null = null;
  function localX(e: MouseEvent): number {
    const r = canvas?.getBoundingClientRect();
    return r ? e.clientX - r.left : 0;
  }
  function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    drag = { px: localX(e), from, to };
    canvas?.setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    const px = localX(e);
    if (drag) {
      const dt = ((px - drag.px) / plotW) * (drag.to - drag.from);
      setRange(drag.from - dt, drag.to - dt);
    }
    hover = px >= LABEL_W ? tAt(px) : null;
  }
  function onUp() {
    drag = null;
  }
  function onWheel(e: WheelEvent) {
    if (e.ctrlKey) {
      e.preventDefault();
      zoom(e.deltaY > 0 ? 1.25 : 0.8, tAt(localX(e)));
    } else if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      e.preventDefault();
      const d = e.shiftKey ? e.deltaY : e.deltaX;
      pan((d / plotW) * (to - from));
    }
  }
  function onKey(e: KeyboardEvent) {
    const span = to - from;
    const act: Record<string, () => void> = {
      ArrowLeft: () => pan(-span * 0.2),
      ArrowRight: () => pan(span * 0.2),
      "+": () => zoom(0.8),
      "=": () => zoom(0.8),
      "-": () => zoom(1.25),
      "0": () => setRange(fullFrom, fullTo),
      Home: () => setRange(fullFrom, fullFrom + span),
      End: () => setRange(fullTo - span, fullTo),
    };
    const f = act[e.key];
    if (f) {
      e.preventDefault();
      f();
    }
  }
  $effect(() => {
    // Коліщатко з Ctrl має не масштабувати сторінку — потрібен не пасивний обробник.
    if (!canvas) return;
    const el = canvas;
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  });

  // ---- Підпис під курсором ----
  const ms = (v: number | null | undefined, d = 1) => (v == null ? "—" : v.toFixed(d));
  const STATES = ["running", "ready", "sync", "page_in", "sleep", "other"] as const;
  function threadAt(th: ThreadLane | null, at: number): string | null {
    if (!th) return null;
    const k = lowerBound(th.detail.length, (i) => {
      const d = th.detail[i];
      return d ? d[0] + d[1] < at : false;
    });
    const d = th.detail[k];
    if (d && d[0] <= at) return tk(`ui.anatomy.main.${STATES[d[2]] ?? "other"}`);
    const b = th.buckets[Math.floor((at - th.start_ms) / th.bucket_ms)];
    if (!b) return null;
    let best = 0;
    b.forEach((v, i) => {
      if (v > (b[best] ?? 0)) best = i;
    });
    const pct = Math.round(((b[best] ?? 0) / th.bucket_ms) * 100);
    return tk("ui.tl.mostly", { state: tk(`ui.anatomy.main.${STATES[best] ?? "other"}`), pct });
  }
  const readout = $derived.by(() => {
    if (hover == null) return null;
    const at = hover;
    const tt = view.frames.t;
    const i = lowerBound(tt.length, (k) => (tt[k] ?? 0) < at);
    const frameIn = (tt[i] ?? Infinity) - (view.frames.ft[i] ?? 0) <= at;
    const st = view.samples.t;
    const si = Math.max(0, lowerBound(st.length, (k) => (st[k] ?? 0) <= at) - 1);
    const pxMs = ((to - from) / plotW) * 3;
    const dpc = view.dpc.filter((d) => d.t <= at + pxMs && d.t + d.us / 1000 >= at - pxMs);
    return {
      t: ((at - fullFrom) / 1000).toFixed(3),
      frame: frameIn
        ? tk("ui.tl.frame", {
            ft: ms(view.frames.ft[i]),
            cpu: ms(view.frames.cpu[i]),
            gpu: ms(view.frames.gpu[i]),
            q: ms(view.frames.queue[i]),
          })
        : tk("ui.tl.no_frame"),
      main: threadAt(view.main, at),
      render: threadAt(view.render, at),
      load: st.length
        ? tk("ui.tl.load", { gpu: ms(view.samples.gpu[si], 0), core: ms(view.samples.core[si], 0) })
        : null,
      // Температура CPU є лише з PawnIO (у старих записах масиву немає).
      cpuTemp: st.length && view.samples.cpu_temp?.[si] != null
        ? tk("ui.tl.cpu_temp", { c: ms(view.samples.cpu_temp[si], 0) })
        : null,
      dpc: dpc.map((d) =>
        tk("ui.tl.dpc", {
          driver: view.drivers[d.driver] || tk("ui.tl.unknown_driver"),
          ms: (d.us / 1000).toFixed(2),
        }),
      ),
    };
  });
</script>

<div class="tl">
  <div class="bar">
    <div class="group" role="group" aria-label={tk("ui.tl.zoom")}>
      <button type="button" class="btn" onclick={() => zoom(0.5)} aria-label={tk("ui.tl.zoom_in")} title={tk("ui.tl.zoom_in")}>+</button>
      <button type="button" class="btn" onclick={() => zoom(2)} aria-label={tk("ui.tl.zoom_out")} title={tk("ui.tl.zoom_out")}>−</button>
      <button type="button" class="btn" onclick={() => setRange(fullFrom, fullTo)}>{tk("ui.tl.all")}</button>
    </div>
    {#if view.spikes.length}
      <div class="group" role="group" aria-label={tk("ui.tl.spikes")}>
        <button type="button" class="btn" onclick={() => step(-1)}>← {tk("ui.tl.prev")}</button>
        <button type="button" class="btn" onclick={worst}>{tk("ui.tl.worst")}</button>
        <button type="button" class="btn" onclick={() => step(1)}>{tk("ui.tl.next")} →</button>
      </div>
    {/if}
    <span class="small dim range mono">
      {((from - fullFrom) / 1000).toFixed(2)}–{((to - fullFrom) / 1000).toFixed(2)} {tk("ui.tl.s")}
    </span>
  </div>

  {#if current}
    <p class="small spike" aria-live="polite">
      {tk("ui.tl.spike", {
        i: selected + 1,
        n: view.spikes.length,
        t: ((current[0] - fullFrom) / 1000).toFixed(2),
        ms: current[1].toFixed(1),
      })}
      {#if causes.length} · <span class="muted">{causes.join("; ")}</span>{/if}
    </p>
  {:else if !view.spikes.length}
    <p class="small muted">{tk("ui.tl.no_spikes")}</p>
  {/if}

  <!-- Полотно керується клавішами (зсув, масштаб): role="application" — інтерактивне, хоч
       Svelte і не вважає цю роль такою. Усі дії продубльовано кнопками вище. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="canvas"
    bind:this={wrap}
    tabindex="0"
    role="application"
    aria-label={tk("ui.tl.aria")}
    aria-describedby={hintId}
    onkeydown={onKey}
  >
    <canvas
      bind:this={canvas}
      style:height="{height}px"
      aria-hidden="true"
      onpointerdown={onDown}
      onpointermove={onMove}
      onpointerup={onUp}
      onpointercancel={onUp}
      onpointerleave={() => (hover = null)}
      ondblclick={(e) => zoom(0.5, tAt(localX(e)))}
    ></canvas>
  </div>

  <div class="readout small mono" class:empty={!readout}>
    {#if readout}
      <span>{readout.t} {tk("ui.tl.s")}</span>
      <span>{readout.frame}</span>
      {#if readout.main}<span>{tk("ui.tl.lane.main")}: {readout.main}</span>{/if}
      {#if readout.render}<span>{tk("ui.tl.lane.render")}: {readout.render}</span>{/if}
      {#if readout.load}<span>{readout.load}</span>{/if}
      {#if readout.cpuTemp}<span>{readout.cpuTemp}</span>{/if}
      {#each readout.dpc as d, i (i)}<span class="danger">{d}</span>{/each}
    {/if}
  </div>
  <p class="small dim" id={hintId}>{tk("ui.tl.hint")}</p>

  <div class="legend small muted">
    <span><i class="sw frame"></i>{tk("ui.tl.legend.frame")}</span>
    <span><i class="sw warn"></i>{tk("ui.tl.legend.spike")}</span>
    {#if hasWork}
      <span><i class="ln cpu"></i>{tk("ui.tl.legend.cpu")}</span>
      <span><i class="ln gpu"></i>{tk("ui.tl.legend.gpu")}</span>
    {/if}
    {#if hasQueue}<span><i class="ln queue"></i>{tk("ui.tl.legend.queue")}</span>{/if}
    {#if view.samples.t.length}
      <span><i class="ln core"></i>{tk("ui.tl.legend.core")}</span>
      <span><i class="ln gpuload"></i>{tk("ui.tl.legend.gpu_util")}</span>
    {/if}
    {#if view.dpc.length}<span><i class="sw danger"></i>{tk("ui.tl.legend.dpc")}</span>{/if}
    {#if view.shader_writes.length}<span><i class="dot warn"></i>{tk("ui.tl.legend.shader")}</span>{/if}
    {#if view.gaps.length}<span><i class="sw gap"></i>{tk("ui.tl.legend.gap")}</span>{/if}
  </div>
  {#if view.main}
    <div class="legend small muted">
      <span>{tk("ui.tl.states")}:</span>
      <span><i class="sw run"></i>{tk("ui.anatomy.main.running")}</span>
      <span><i class="sw warn"></i>{tk("ui.anatomy.main.ready")}</span>
      <span><i class="sw queue"></i>{tk("ui.anatomy.main.sync")}</span>
      <span><i class="sw danger"></i>{tk("ui.anatomy.main.page_in")}</span>
      <span><i class="sw sleep"></i>{tk("ui.anatomy.main.sleep")}</span>
    </div>
    <p class="small dim">{tk("ui.tl.detail_note")}</p>
  {/if}
</div>

<style>
  .tl {
    display: grid;
    gap: calc(var(--space) * 3);
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
  .group {
    display: flex;
    gap: calc(var(--space) * 1);
  }
  .btn {
    min-width: 28px;
    padding: calc(var(--space) * 1) calc(var(--space) * 2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: border-color var(--dur-fast) var(--ease);
  }
  .btn:hover {
    border-color: var(--accent-from);
  }
  .btn:active {
    transform: scale(0.98);
  }
  .btn:focus-visible,
  .canvas:focus-visible {
    outline: 2px solid var(--accent-from);
    outline-offset: 2px;
  }
  .range {
    margin-left: auto;
  }
  .canvas {
    width: 100%;
  }
  canvas {
    display: block;
    width: 100%;
    cursor: grab;
    touch-action: none;
    border-radius: var(--radius-sm);
  }
  canvas:active {
    cursor: grabbing;
  }
  .readout {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 1) calc(var(--space) * 4);
    min-height: 18px;
  }
  .spike {
    margin: 0;
    color: var(--warn);
  }
  .danger {
    color: var(--danger);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: calc(var(--space) * 1) calc(var(--space) * 4);
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: calc(var(--space) * 2);
  }
  .sw {
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
  .ln {
    width: 14px;
    height: 2px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }
  .frame {
    background: var(--a2);
  }
  .cpu {
    background: var(--a1);
  }
  .gpu,
  .core {
    background: var(--gpu);
  }
  .gpuload {
    background: var(--ok);
  }
  .queue {
    background: var(--queue);
  }
  .run {
    background: var(--thread-work);
  }
  .sleep {
    background: var(--thread-sleep);
  }
  .warn {
    background: var(--warn);
  }
  .sw.danger {
    background: var(--danger);
  }
  .gap {
    background: var(--border);
  }
  p {
    margin: 0;
  }
</style>
