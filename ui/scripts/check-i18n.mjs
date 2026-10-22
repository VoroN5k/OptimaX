// Перевірка: кожен ключ тексту, використаний в інтерфейсі, є в каталозі uk.json.
// Запуск: npm run check:i18n
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const catalog = JSON.parse(readFileSync(new URL("../../crates/core/locales/uk.json", import.meta.url), "utf8"));
const files = [];
(function walk(dir) {
  for (const f of readdirSync(dir)) {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) {
      if (f !== "bindings") walk(p);
    } else if (/\.(svelte|ts)$/.test(f)) files.push(p);
  }
})(new URL("../src", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1"));

const used = new Set();
for (const f of files) {
  const src = readFileSync(f, "utf8");
  for (const m of src.matchAll(/["'`]((?:ui|priority|service|tweak|gm|audit|diag|ab)\.[a-z0-9_.]+)["'`]/g)) used.add(m[1]);
}
// Динамічні ключі: розгортаємо за відомими значеннями.
const screens = ["analysis", "optimization", "games", "diagnosis", "hardware", "laptop", "network", "cleanup", "apps", "startup", "appearance", "backups", "settings"];
for (const s of screens) used.add(`ui.nav.${s}`);
for (const s of ["laptop", "network", "cleanup", "apps", "startup", "appearance"]) used.add(`ui.placeholder.${s}`);
for (const k of ["power_scheme", "process_priority", "service", "standby_list"]) used.add(`ui.target.${k}`);
for (const k of ["pending", "applied", "performed", "rolled_back", "failed"]) used.add(`ui.backups.state.${k}`);
for (const k of ["WSearch", "SysMain"]) used.add(`ui.games.service.${k}`);
for (const k of ["waiting_for_game", "waiting_for_hotkey", "warmup", "recording", "paused", "done"]) {
  used.add(`ui.rec.phase.${k}`);
  used.add(`ui.rec.hint.${k}`);
}
for (const k of ["completed", "hotkey", "game_exited", "cancelled", "wait_timeout"]) {
  used.add(`ui.rec.end.${k}`);
  used.add(`ui.rec.not_started.${k}`);
}

const missing = [...used].filter((k) => !(k in catalog)).sort();
if (missing.length) {
  console.error(`Немає в uk.json (${missing.length}):\n` + missing.join("\n"));
  process.exit(1);
}
console.log(`i18n OK: ${used.size} ключів`);
