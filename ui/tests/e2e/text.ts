// Тексти для перевірок — з того самого каталогу й тим самим рендером, що в інтерфейсі.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { setCatalog, tk } from "../../src/lib/i18n";

setCatalog(
  JSON.parse(readFileSync(fileURLToPath(new URL("../../../crates/core/locales/uk.json", import.meta.url)), "utf8")),
);

export { tk };
