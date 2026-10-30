// Генерує release/latest.json для оновлення з програми (tauri-plugin-updater).
// Запуск після `npm run app:bundle` з ключем підпису: npm run release:latest
// Формат — https://v2.tauri.app/plugin/updater/ (version, notes, pub_date, platforms.windows-x86_64).
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const conf = JSON.parse(readFileSync(join(root, "crates", "app", "tauri.conf.json"), "utf8"));
const version = conf.version;
const sigPath = join(root, "target", "release", "bundle", "nsis", `OptimaX_${version}_x64-setup.exe.sig`);
if (!existsSync(sigPath)) {
  console.error(`Немає ${sigPath}.\nЗберіть інсталятор із ключем підпису (TAURI_SIGNING_PRIVATE_KEY) — див. docs/RELEASE.md.`);
  process.exit(1);
}

// Що нового — розділ CHANGELOG.md для цієї версії (до наступного «## [»).
const changelog = readFileSync(join(root, "CHANGELOG.md"), "utf8");
const start = changelog.indexOf(`## [${version}]`);
let notes = "";
if (start >= 0) {
  const body = changelog.slice(changelog.indexOf("\n", start) + 1);
  const end = body.indexOf("\n## [");
  notes = (end >= 0 ? body.slice(0, end) : body).trim();
} else {
  console.warn(`У CHANGELOG.md немає розділу «## [${version}]» — опис оновлення буде порожнім.`);
}

const latest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      // Вміст .sig, а не шлях (так вимагає плагін).
      signature: readFileSync(sigPath, "utf8").trim(),
      // Ім'я файлу має збігатися з тим, що завантажено в реліз (крок 5 docs/RELEASE.md).
      url: `https://github.com/VoroN5k/OptimaX/releases/download/v${version}/OptimaX-${version}-win-x64-setup.exe`,
    },
  },
};

const outDir = join(root, "release");
mkdirSync(outDir, { recursive: true });
const out = join(outDir, "latest.json");
writeFileSync(out, JSON.stringify(latest, null, 2) + "\n");
console.log(`Збережено ${out} (версія ${version}).`);
