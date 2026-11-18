// Очікування, поки гра з бібліотеки з'явиться на екрані. Опитує бекенд у власному потоці
// (`library_detect_start`), а сюди приходять лише події `library-detect`: поки гра на весь екран,
// WebView2 може пригальмовувати таймери інтерфейсу, тож на них визначення не покладається.
import { api, on } from "./api";
import type { DetectState } from "./bindings/DetectState";

/** Після цього часу без результату — підказка вибрати процес вручну. */
export const DETECT_SLOW_MS = 60_000;

/** Що повідомляти екрану. */
export interface DetectHandlers {
  /** Гру визначено. */
  onFound: (exe: string) => void;
  /** Помилка бекенду чи гра без теки — очікування зупинено. */
  onError: (e: unknown) => void;
  /** Ще чекаємо; `skipped` — exe, які в цій сесії пропускаються (закрились посеред запису). */
  onWaiting?: (skipped: string[]) => void;
  /** Минуло [`DETECT_SLOW_MS`] без результату (очікування триває). */
  onSlow?: () => void;
}

/** Починає визначення. Повертає функцію скасування (повідомляє бекенд). */
export function detectGame(id: string, h: DetectHandlers): () => void {
  let active = true;
  const slow = setTimeout(() => active && h.onSlow?.(), DETECT_SLOW_MS);

  function handle(s: DetectState) {
    if (!active) return;
    if (s.state === "found") {
      stop();
      h.onFound(s.exe);
    } else if (s.state === "no_folder") {
      cancel();
      h.onError(new Error("no_folder"));
    } else {
      h.onWaiting?.(s.skipped);
    }
  }

  const unlisten = on("library-detect", handle);
  void unlisten
    .then(() => api.libraryDetectStart(id))
    .then(handle)
    .catch((e) => {
      if (!active) return;
      stop();
      h.onError(e);
    });

  function stop() {
    active = false;
    clearTimeout(slow);
    void unlisten.then((u) => u()).catch(() => undefined);
  }

  function cancel() {
    if (!active) return;
    stop();
    void api.libraryDetectCancel().catch(() => undefined);
  }

  return cancel;
}
