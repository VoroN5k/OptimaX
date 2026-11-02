// Підроблений бекенд для UI-тестів: замість Tauri — `window.__TAURI_INTERNALS__` з відповідями
// з `fixtures.ts` (так само влаштований офіційний `mockIPC` з `@tauri-apps/api/mocks`).
// Інтерфейс отримує ті самі типи, що й від справжнього бекенду, тож тести ловлять зміни
// в розмітці, текстах і логіці показу, але не в Rust-коді (для нього — `cargo test`).
import type { Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defaults, type Responses } from "./fixtures";

/** Справжній каталог текстів (як `get_catalog` бекенду). */
const catalog: Record<string, string> = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../../crates/core/locales/uk.json", import.meta.url)), "utf8"),
);

export interface Call {
  cmd: string;
  args: unknown;
}

/** Ставить підроблений бекенд до завантаження сторінки. `overrides` — відповіді команд. */
export async function mockBackend(page: Page, overrides: Partial<Responses> = {}): Promise<void> {
  const responses: Record<string, { ok: unknown } | { err: unknown }> = {};
  for (const [cmd, value] of Object.entries({ ...defaults(), ...overrides, get_catalog: catalog })) {
    responses[cmd] = { ok: value };
  }
  await page.addInitScript((responses) => {
    type Cb = (data: unknown) => void;
    const w = window as unknown as Record<string, unknown>;
    const calls: { cmd: string; args: unknown }[] = [];
    const unmocked: string[] = [];
    const listeners = new Map<string, number[]>();
    const callbacks = new Map<number, Cb>();
    let nextId = 1;
    w.__calls = calls;
    w.__unmocked = unmocked;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, id: number) => callbacks.delete(id),
    };
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
      callbacks,
      transformCallback(cb: Cb, once = false) {
        const id = nextId++;
        callbacks.set(id, (data) => {
          if (once) callbacks.delete(id);
          cb?.(data);
        });
        return id;
      },
      unregisterCallback(id: number) {
        callbacks.delete(id);
      },
      runCallback(id: number, data: unknown) {
        callbacks.get(id)?.(data);
      },
      async invoke(cmd: string, args: Record<string, unknown>) {
        calls.push({ cmd, args });
        if (cmd === "plugin:event|listen") {
          const event = String(args.event);
          const list = listeners.get(event) ?? [];
          list.push(Number(args.handler));
          listeners.set(event, list);
          return args.handler;
        }
        if (cmd.startsWith("plugin:")) return null;
        // Збережені налаштування повертаються при наступному читанні, як у бекенді.
        if (cmd === "set_settings") responses.get_settings = { ok: args.settings };
        const r = responses[cmd];
        if (!r) {
          unmocked.push(cmd);
          return null;
        }
        if ("err" in r) throw r.err;
        return structuredClone(r.ok);
      },
    };
    /** Подія бекенду (як `app.emit`). */
    w.__emit = (event: string, payload: unknown) => {
      for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
    };
  }, responses);
}

/** Виклики команд бекенду (без службових `plugin:`). */
export async function calls(page: Page): Promise<Call[]> {
  const all = await page.evaluate(() => (window as unknown as { __calls: Call[] }).__calls);
  return all.filter((c) => !c.cmd.startsWith("plugin:"));
}

/** Команди, для яких тест не дав відповіді (отримали `null`). */
export async function unmocked(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as unknown as { __unmocked: string[] }).__unmocked);
}

/** Подія бекенду в сторінку. */
export async function emit(page: Page, event: string, payload: unknown): Promise<void> {
  await page.evaluate(
    ([e, p]) => (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(e as string, p),
    [event, payload] as const,
  );
}
