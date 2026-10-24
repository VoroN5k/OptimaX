// Форматування чисел для показу.

export function mib(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  return `${Math.round(bytes / 1048576).toLocaleString("uk-UA")} МіБ`;
}

export function gib(bytes: number | null | undefined, digits = 1): string {
  if (bytes == null) return "—";
  return `${(bytes / 1073741824).toFixed(digits)} ГБ`;
}

export function pct(v: number | null | undefined, digits = 0): string {
  if (v == null || !Number.isFinite(v)) return "—";
  return `${v.toFixed(digits)}%`;
}

export function rate(bytesPerSec: number): string {
  if (bytesPerSec >= 1048576) return `${(bytesPerSec / 1048576).toFixed(1)} МіБ/с`;
  return `${Math.round(bytesPerSec / 1024)} КіБ/с`;
}

export function time(unixMs: number): string {
  return new Date(unixMs).toLocaleTimeString("uk-UA", { hour12: false });
}

export function dateTime(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleString("uk-UA", { hour12: false });
}

/** Розмір з автоматичною одиницею (1024-кратні, як у Провіднику). */
export function size(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  const units = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i >= 3 ? 1 : 0)} ${units[i]}`;
}

export function date(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleDateString("uk-UA");
}
