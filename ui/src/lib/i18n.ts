// Локалізація: бекенд віддає тексти як {key, args}, рендер — тут, за тим самим каталогом
// і тими самими правилами, що й у Rust (optimax-core::i18n): `{name}` і `{n|одна|дві|багато}`.
import type { Text } from "./bindings/Text";

let catalog: Record<string, string> = {};

/** Встановлює каталог (один раз при старті). */
export function setCatalog(entries: Record<string, string>): void {
  catalog = entries;
}

/** Індекс множинної форми для української: 0 — «1 день», 1 — «2 дні», 2 — «5 днів». */
export function pluralIndexUk(n: number): number {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return 0;
  if (m10 >= 2 && m10 <= 4 && !(m100 >= 12 && m100 <= 14)) return 1;
  return 2;
}

function resolve(placeholder: string, args: Record<string, string>): string | null {
  const [name, ...forms] = placeholder.split("|");
  const value = name !== undefined ? args[name] : undefined;
  if (value === undefined) return null;
  if (forms.length === 0) return value;
  const n = Number(value.trim().replace(/^-/, ""));
  if (!Number.isInteger(n)) return null;
  return forms[pluralIndexUk(n)] ?? forms[forms.length - 1] ?? null;
}

/** Рендер шаблону. Невідомий ключ повертається як є, щоб пропуск було видно. */
export function renderKey(key: string, args: Record<string, string> = {}): string {
  const template = catalog[key];
  if (template === undefined) return key;
  return template.replace(/\{([^{}]*)\}/g, (whole, inner: string) => resolve(inner, args) ?? whole);
}

/** Рендер тексту з бекенду. */
export function t(text: Text | null | undefined): string {
  if (!text) return "";
  return renderKey(text.key, text.args ?? {});
}

/** Текст інтерфейсу за ключем (ключі UI — `ui.*` у тому самому каталозі). */
export function tk(key: string, args: Record<string, string | number> = {}): string {
  const strArgs: Record<string, string> = {};
  for (const [k, v] of Object.entries(args)) strArgs[k] = String(v);
  return renderKey(key, strArgs);
}
