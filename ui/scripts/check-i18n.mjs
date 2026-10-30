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
  for (const m of src.matchAll(/["'`]((?:ui|priority|service|tweak|gm|audit|diag|ab|toast|upscale\.(?:filter|error))\.[a-z0-9_.]+)["'`]/g)) used.add(m[1]);
}
// Динамічні ключі: розгортаємо за відомими значеннями.
const screens = ["analysis", "optimization", "games", "diagnosis", "hardware", "laptop", "network", "upscale", "cleanup", "apps", "startup", "appearance", "backups", "settings"];
for (const s of screens) used.add(`ui.nav.${s}`);
for (const s of ["laptop", "network", "cleanup", "apps", "startup", "appearance"]) used.add(`ui.placeholder.${s}`);
for (const s of ["user_run", "machine_run", "machine_run32", "user_folder", "common_folder"]) used.add(`ui.startup.source.${s}`);
for (const k of ["system", "unknown_flag"]) {
  used.add(`ui.startup.lock.${k}`);
  used.add(`ui.startup.lock_short.${k}`);
}
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

for (const f of ["nearest", "bilinear", "lanczos", "fsr", "nis"]) {
  used.add(`upscale.filter.${f}`);
  used.add(`ui.up.filter_hint.${f}`);
}
for (const f of ["aspect", "integer", "stretch"]) used.add(`ui.up.fit.${f}`);
for (const k of ["ethernet", "wifi", "virtual", "other"]) used.add(`ui.net.kind.${k}`);
for (const b of ["ghz2_4", "ghz5", "ghz6"]) used.add(`ui.net.band.${b}`);
for (const v of ["safe", "consequences", "protected", "unknown"]) {
  used.add(`ui.clean.verdict.${v}`);
  used.add(`ui.clean.verdict_hint.${v}`);
}
for (const z of ["system", "programs", "user_files"]) used.add(`ui.clean.zone.${z}`);
for (const c of ["temp", "windows_update", "drivers", "browser_caches", "dev_caches", "shader_caches", "crash_dumps", "system", "unknown"]) used.add(`ui.clean.cat.${c}`);
for (const a of ["delete", "quarantine"]) used.add(`ui.clean.action.${a}`);
for (const r of ["clean", "restore", "purge", "dism", "hibernate_off", "hibernate_on"]) used.add(`ui.clean.run.${r}`);
for (const o of ["deleted", "quarantined", "restored", "purged", "skipped"]) used.add(`ui.clean.outcome.${o}`);
for (const t of ["now", "growth", "quarantine", "journal"]) used.add(`ui.clean.tab.${t}`);
for (const k of ["dism", "hibernate"]) for (const s of ["title", "text", "command", "confirm", "done"]) used.add(`ui.clean.${k}_${s}`);
for (const k of ["restore", "purge"]) used.add(`ui.clean.q_${k}_log`);
for (const k of ["dism", "hibernate"]) used.add(`ui.clean.running_${k}`);

const missing = [...used].filter((k) => !(k in catalog)).sort();
if (missing.length) {
  console.error(`Немає в uk.json (${missing.length}):\n` + missing.join("\n"));
  process.exit(1);
}
console.log(`i18n OK: ${used.size} ключів`);
