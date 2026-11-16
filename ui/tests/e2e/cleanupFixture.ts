// Реалістичний скан диска для UI-тестів «Очищення»: кожен вердикт і кілька категорій.
import type { DriveView } from "../../src/lib/bindings/DriveView";
import type { Item } from "../../src/lib/bindings/Item";
import type { ScanView } from "../../src/lib/bindings/ScanView";

const GIB = 1024 ** 3;
const MIB = 1024 ** 2;

export const cDrive: DriveView = {
  root: "C:\\",
  label: "System",
  total_bytes: 930 * GIB,
  free_bytes: 120 * GIB,
  is_system: true,
  removable: false,
};

function item(
  path: string,
  category: Item["category"],
  verdict: Item["verdict"],
  bytes: number,
  files: number,
  rule: { id: string; description: string; consequences: string } | null,
  method: Item["method"] = "delete",
): Item {
  const selectable = verdict === "safe" || verdict === "consequences";
  return {
    path,
    category,
    verdict,
    method,
    bytes,
    files,
    eligible_bytes: selectable ? bytes : 0,
    eligible_files: selectable ? files : 0,
    selectable: selectable && method === "delete",
    nothing_eligible: false,
    rule: rule
      ? {
          ...rule,
          regenerates: verdict !== "protected",
          source: "https://learn.microsoft.com/",
          verified_on: "Windows 11 24H2, 2026-10-30",
          manual_hint: null,
          demoted: false,
          min_age_days: null,
        }
      : null,
    zone: null,
    remainder: false,
    small_dirs: 0,
    top_subdirs: [],
    unreadable_dirs: 0,
  };
}

export const cleanScan: ScanView = {
  drive: cDrive,
  safe_bytes: 9.4 * GIB,
  consequences_bytes: 3.1 * GIB,
  suggest_move: false,
  rule_issues: [],
  report: {
    drive: "C:\\",
    full: true,
    started_unix: 1_793_600_000,
    duration_ms: 18_400,
    scanned_bytes: 810 * GIB,
    unreadable_dirs: 0,
    cancelled: false,
    items: [
      item("C:\\Users\\me\\AppData\\Local\\Temp", "temp", "safe", 4.2 * GIB, 18_340, {
        id: "user_temp",
        description: "Тимчасові файли програм користувача.",
        consequences: "Нічого: програми створять їх знову за потреби.",
      }),
      item("C:\\Windows\\SoftwareDistribution\\Download", "windows_update", "safe", 2.6 * GIB, 412, {
        id: "wu_download",
        description: "Завантажені й уже встановлені оновлення Windows.",
        consequences: "Нічого: Windows завантажить потрібне знову.",
      }),
      item("C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache", "browser_caches", "safe", 1.8 * GIB, 9_120, {
        id: "chrome_cache",
        description: "Кеш сторінок Chrome.",
        consequences: "Сайти один раз завантажаться повільніше.",
      }),
      item("C:\\Users\\me\\AppData\\Local\\NVIDIA\\DXCache", "shader_caches", "consequences", 3.1 * GIB, 2_210, {
        id: "nvidia_dxcache",
        description: "Скомпільовані шейдери NVIDIA (DirectX).",
        consequences: "Перші запуски ігор підвисатимуть, поки шейдери не скомпілюються знову.",
      }),
      item("C:\\Users\\me\\AppData\\Local\\CrashDumps", "crash_dumps", "safe", 820 * MIB, 14, {
        id: "user_crash_dumps",
        description: "Дампи падінь програм.",
        consequences: "Не буде чого надіслати розробникам про старі падіння.",
      }),
      item("C:\\Windows\\System32\\DriverStore\\FileRepository", "drivers", "protected", 6.4 * GIB, 21_400, {
        id: "driver_store",
        description: "Сховище драйверів Windows.",
        consequences: "Видаляти вручну не можна — лише штатними засобами.",
      }, "manual_only"),
    ],
    unknown: [
      item("C:\\Games\\old-mods", "unknown", "unknown", 11.2 * GIB, 3_400, null),
      item("C:\\Users\\me\\Videos\\Captures", "unknown", "unknown", 7.9 * GIB, 61, null),
    ],
    protected_zones: [],
  },
};
