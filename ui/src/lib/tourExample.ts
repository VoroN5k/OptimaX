// Приклад результату для знайомства (розділ 3), коли власного запису ще немає. Це НЕ виміряні
// дані: екран показує його лише під час знайомства, з помітною позначкою «Приклад — не ваш ПК»,
// і нікуди не зберігає. Тексти — справжні шаблони вердиктів із каталогу.
import type { DiagnosisResult } from "./bindings/DiagnosisResult";
import type { Finding } from "./bindings/Finding";

const finding = (f: Partial<Finding> & Pick<Finding, "id" | "title" | "detail">): Finding => ({
  category: "gpu",
  impact: "info",
  confidence: 80,
  advice: null,
  evidence: [],
  ...f,
});

export const EXAMPLE_RESULT: DiagnosisResult = {
  diagnosis: {
    game: "game.exe",
    stats: {
      frames: 4200,
      duration_ms: 30000,
      avg_fps: 140,
      low_1pct_fps: 98,
      low_01pct_fps: 71,
      median_frametime_ms: 7.1,
      p99_frametime_ms: 10.4,
      max_frametime_ms: 24,
      spikes: 3,
    },
    bottleneck: "gpu",
    verdicts: [
      {
        finding: finding({
          id: "summary.gpu",
          category: "summary",
          title: { key: "diag.summary.gpu.title", args: { game: "game.exe" } },
          detail: {
            key: "diag.summary.gpu.detail",
            args: { seconds: "30", frames: "4200", avg: "140.0", low1: "98.0", low01: "71.0" },
          },
        }),
        expected_gain: null,
      },
      {
        finding: finding({
          id: "gpu_bound",
          title: { key: "diag.gpu_bound.title", args: { share: "88" } },
          detail: { key: "diag.gpu_bound.detail", args: {} },
          advice: { key: "diag.gpu_bound.advice", args: {} },
        }),
        expected_gain: null,
      },
      {
        finding: finding({
          id: "stutter.isolated",
          category: "stutter",
          confidence: 90,
          title: { key: "diag.stutter.isolated.title", args: { spikes: "3", max: "24", first: "2.1" } },
          detail: { key: "diag.stutter.isolated.detail", args: { first: "2.1" } },
        }),
        expected_gain: null,
      },
    ],
    chart: null,
    frame_gen: null,
  },
  display_hz: 144,
  presentmon_messages: [],
  cancelled: false,
  counted_s: 30,
  end_reason: "completed",
};
