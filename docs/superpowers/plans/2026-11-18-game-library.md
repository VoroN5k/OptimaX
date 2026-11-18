# Бібліотека ігор — план реалізації

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** гра вибирається зі списку встановлених ігор Steam / Epic / GOG / Battle.net, а процес
гри програма визначає сама за текою встановлення.

**Architecture:** новий модуль `library` у крейті `optimax-gamemode`: чисті парсери маніфестів,
читання через вузькі трейти (`LibraryFs`, наявний `RegistryApi`), стан користувача в
`library.json`, чиста функція вибору процесу гри. Tauri-команди в `crates/app/src/commands/library.rs`,
UI — компонент `GamePicker.svelte` у «Діагностиці» та «Іграх».

**Tech Stack:** Rust (serde, serde_json, windows, sysinfo — усе вже є), Tauri 2, Svelte 5, ts-rs.

**Spec:** `docs/superpowers/specs/2026-11-18-game-library-design.md`

## Global Constraints

- Жодних нових залежностей (VDF і base64 — власні мінімальні функції).
- Жодних мережевих запитів; обкладинки лише з `<корінь Steam>\appcache\librarycache\<appid>\library_600x900.jpg`.
- Без `unwrap`/`expect`/`panic` у продакшн-коді; `#![forbid(unsafe_code)]` у `gamemode`
  (unsafe лише в `crates/core/src/registry/windows.rs` з `// SAFETY:`).
- Публічні типи й функції — з doc-коментарями українською.
- Типи для UI — `#[cfg_attr(feature = "ts", derive(ts_rs::TS))]` + рядок у списку `export!` у
  `crates/app/src/bindings.rs`; генерація — `cargo test -p optimax-app`.
- Тексти UI — ключі в `crates/core/locales/uk.json`, у коді лише `tk("…")`; перевірка `npm run check:i18n`.
- Стиль — токени `ui/src/styles/tokens.css`, компоненти з `ui/src/components/`, `docs/DESIGN.md`.
- Фронтенд шляхів до файлів не передає: обкладинка запитується за `id` гри.
- Git робить лише користувач: замість кроку «Commit» — зупинка з пропозицією повідомлення коміту.
- Перевірка кожного завдання: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p <крейт>`.

## Review Focus

1. **Лаунчер проміжний у теці гри** (Genshin: `launcher_epic.exe` лежить у теці гри) — програма
   не має «впізнати» лаунчер як гру; чекає, поки на передньому плані буде найважчий процес теки
   (тест `pick_ignores_light_foreground_launcher`, завдання 5).
2. **Схожі теки** `F:\Games\Foo` і `F:\Games\FooBar`, змішані `/` і `\`, регістр, `..` у шляху
   процесу — не збіг (тести `inside_dir_*`, завдання 5).
3. **Відключений диск / видалена тека гри / битий маніфест** — список будується з решти, проблема
   потрапляє в `problems`, без паніки (тести `scan_*`, завдання 4).
4. **Повторний скан не стирає вибір людини** (прихована гра, визначений exe) і гра, що зникла
   з лаунчера, не ламає `library.json` (тест `store_survives_rescan`, завдання 4).
5. **Однакова гра з двох джерел / дублікати Epic** — у списку один запис (тести
   `epic_dedups_same_install`, `scan_dedups_by_install_dir`, завдання 2 і 4).

---

### Task 1: Парсер VDF і маніфести Steam

**Files:**
- Create: `crates/gamemode/src/library/mod.rs`, `crates/gamemode/src/library/vdf.rs`, `crates/gamemode/src/library/steam.rs`
- Modify: `crates/gamemode/src/lib.rs` (додати `pub mod library;`)
- Test: у тих самих файлах, `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `vdf::Node` — `enum Node { Str(String), Map(Vec<(String, Node)>) }` (порядок ключів зберігається).
  - `vdf::parse(text: &str) -> Result<Node, LibraryError>` — корінь `Map`.
  - `Node::get(&self, key: &str) -> Option<&Node>` (без урахування регістру), `Node::str(&self) -> Option<&str>`.
  - `steam::library_paths(libraryfolders: &str) -> Result<Vec<String>, LibraryError>` — значення `path` кожної бібліотеки, `\\` → `\`.
  - `steam::AppManifest { appid: String, name: String, installdir: String }`;
    `steam::parse_appmanifest(text: &str) -> Result<AppManifest, LibraryError>`.
  - `steam::EXCLUDED_APPIDS: &[&str] = &["228980"]` (Steamworks Common Redistributables, звірено 2026-11-18).
  - `library::LibraryError` (thiserror): `Parse { what: &'static str, reason: String }`, `Io(String)`.

- [ ] **Step 1: Тести VDF** — `parse_nested_maps` (зразок `libraryfolders` із двома бібліотеками
  `"D:\\Steam"` і `"E:\\SteamLibrary"`, вкладений `"apps"`), `parse_skips_line_comments` (`// …`),
  `parse_unescapes_backslashes` (`"D:\\Steam"` → `D:\Steam`), `parse_rejects_unclosed_brace` (→ `Err`),
  `parse_rejects_unterminated_string` (→ `Err`), `get_is_case_insensitive` (`"AppState"` / `"appstate"`).
- [ ] **Step 2:** `cargo test -p optimax-gamemode library::vdf` — FAIL (немає модуля).
- [ ] **Step 3:** Реалізувати `vdf.rs`: токенізатор рядків у лапках з екрануванням `\\` `\"` `\n` `\t`,
  `{` `}`, коментарі `//` до кінця рядка; рекурсивний розбір із лімітом глибини 32 (більше → `Err`).
- [ ] **Step 4: Тести Steam** — `library_paths_reads_all_folders` (→ `["D:\Steam", "E:\SteamLibrary"]`),
  `appmanifest_reads_fields` (зразок у форматі `appmanifest_1293230.acf` без `LastOwner`: appid
  `1293230`, name `House of Detention`, installdir `House of Detention`), `appmanifest_missing_name_is_error`.
- [ ] **Step 5:** Реалізувати `steam.rs`; `cargo test -p optimax-gamemode library` — PASS.
- [ ] **Step 6:** fmt + clippy; зупинка: запропонувати коміт `feat(gamemode): VDF parser and Steam manifests`.

### Task 2: Маніфести Epic

**Files:**
- Create: `crates/gamemode/src/library/epic.rs`
- Test: у файлі

**Interfaces:**
- Produces:
  - `epic::EpicGame { app_name: String, name: String, install_dir: String, exe_hint: Option<String> }`.
  - `epic::parse_item(text: &str) -> Result<Option<EpicGame>, LibraryError>` — `None` для не-гри.
  - `epic::dedup(games: Vec<EpicGame>) -> Vec<EpicGame>` — за `app_name` і за `install_dir` (без регістру).

- [ ] **Step 1: Тести** — `item_game_is_parsed` (`AppCategories: ["public","games","applications"]`,
  `DisplayName: "Red Dead Redemption 2"`, `MainWindowProcessName: "RDR2.exe"` → `exe_hint == Some("RDR2.exe")`),
  `item_empty_main_window_has_no_hint` (`""` → `None`), `item_engine_is_skipped`
  (`["engines/ue5","engines"]` → `Ok(None)`), `item_plugin_is_skipped`, `item_incomplete_is_skipped`
  (`bIsIncompleteInstall: true`), `item_launch_executable_is_never_hint` (`LaunchExecutable: "PlayRDR2.exe"`,
  порожній `MainWindowProcessName` → `exe_hint == None`), `item_broken_json_is_error`,
  `epic_dedups_same_install` (два записи «Unreal Engine»-стилю з однаковим `InstallLocation` → один).
- [ ] **Step 2:** FAIL. **Step 3:** реалізувати через `serde_json` (`#[serde(default)]` на всіх полях).
  **Step 4:** PASS. **Step 5:** fmt + clippy; зупинка: коміт `feat(gamemode): Epic manifests`.

### Task 3: Підключі реєстру, GOG і Battle.net

**Files:**
- Modify: `crates/core/src/registry.rs` (трейт + `MemRegistry`), `crates/core/src/registry/windows.rs`,
  `crates/tweaks/src/engine/tests.rs:307` (`IgnoringRegistry`)
- Create: `crates/gamemode/src/library/registry.rs`
- Test: `crates/core/src/registry.rs` (мок), `crates/core/src/registry/windows.rs` (реальний ключ, `#[cfg(windows)]`), `library/registry.rs`

**Interfaces:**
- Produces:
  - `RegistryApi::subkey_names(&self, hive: RegHive, path: &str) -> Result<Vec<String>, CoreError>` —
    порожньо, якщо ключа немає (як `value_names`). **Обов'язковий** метод (без реалізації за замовчуванням).
  - `MemRegistry`: підключі виводяться з шляхів уже записаних значень (безпосередні нащадки `path`).
  - `library::registry::RegGame { key: String, name: String, install_dir: String, exe_hint: Option<String> }`.
  - `gog_games(reg: &dyn RegistryApi) -> Result<Vec<RegGame>, CoreError>` — `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>`:
    `gameName`, `path`, `exeFile` (→ `exe_hint`); `key = "gog:<id>"`.
  - `battlenet_games(reg: &dyn RegistryApi) -> Result<Vec<RegGame>, CoreError>` — обидві гілки
    `SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall` і `SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`
    у `HKLM`; `Publisher` містить `Blizzard` або `Activision`; `DisplayName`, `InstallLocation`;
    `exe_hint` — ім'я файлу з `DisplayIcon` (без `,0` в кінці, якщо є); пропуск запису, де ім'я файлу
    `DisplayIcon` = `Battle.net.exe` (без регістру) або `InstallLocation` порожній; `key = "bnet:<ім'я підключа>"`.

- [ ] **Step 1: Тести мока** — `mem_subkeys_lists_direct_children` (значення в `A\B\x` і `A\C\D\y` → `subkey_names("A") == ["B","C"]`),
  `mem_subkeys_missing_key_is_empty`.
- [ ] **Step 2:** FAIL → додати метод у трейт, `MemRegistry`, `IgnoringRegistry` (делегує), Windows-реалізацію
  через `RegEnumKeyExW` (з тим самим `KEY_WOW64_64KEY`, що й `names_at`) → PASS.
- [ ] **Step 3: Тест Windows** — `subkey_names_lists_real_key`: `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion` містить `Uninstall`.
- [ ] **Step 4: Тести GOG / Battle.net на `MemRegistry`** — `gog_reads_game` (gameName «Kerbal Space Program»,
  path `F:\Games\Kerbal Space Program`, exeFile `KSP_x64.exe`), `gog_missing_root_is_empty`,
  `bnet_reads_blizzard_game` (Publisher `Activision`, `DisplayIcon` `F:\Call of Duty\_retail_\cod.exe` → hint `cod.exe`),
  `bnet_skips_launcher` (`DisplayIcon` `E:\Battle.net\Battle.net.exe`), `bnet_skips_other_publishers`,
  `bnet_strips_icon_index` (`…\cod.exe,0` → `cod.exe`).
- [ ] **Step 5:** реалізувати, PASS; `cargo test -p optimax-core -p optimax-tweaks -p optimax-gamemode`.
- [ ] **Step 6:** fmt + clippy; зупинка: коміт `feat(gamemode): GOG and Battle.net from registry`.

### Task 4: Скан бібліотеки й стан користувача

**Files:**
- Create: `crates/gamemode/src/library/scan.rs`, `crates/gamemode/src/library/store.rs`, `crates/gamemode/src/library/fs.rs`
- Modify: `crates/gamemode/src/library/mod.rs` (реекспорт), `crates/gamemode/src/lib.rs` (`pub use library::{…}`)
- Test: у файлах

**Interfaces:**
- Consumes: завдання 1–3.
- Produces:
  - `fs::LibraryFs` (trait, `Send + Sync`): `read_to_string(&self, path: &Path) -> std::io::Result<String>`,
    `is_dir(&self, path: &Path) -> bool`, `is_file(&self, path: &Path) -> bool`,
    `list_files(&self, dir: &Path, ext: &str) -> std::io::Result<Vec<PathBuf>>` (лише файли з розширенням, без рекурсії),
    `read_bytes(&self, path: &Path, max: u64) -> std::io::Result<Vec<u8>>` (більше `max` → помилка).
    `fs::StdFs` — реалізація на `std::fs` (`symlink_metadata`: reparse point — не тека/не файл). `fs::MemFs` — для тестів.
  - `Launcher` (ts): `Steam | Epic | Gog | Battlenet | Manual`, serde `snake_case`.
  - `GameEntry` (ts): `{ id: String, name: String, launcher: Launcher, install_dir: String, exe: Option<String>, exe_hint: Option<String>, has_cover: bool, hidden: bool }`.
  - `LibraryProblem` (ts): `{ launcher: Launcher, what: String, reason: String }`.
  - `LibraryReport` (ts): `{ games: Vec<GameEntry>, problems: Vec<LibraryProblem> }`.
  - `scan::Sources { steam_root: Option<PathBuf>, epic_manifests: PathBuf }`;
    `scan::steam_root(reg: &dyn RegistryApi) -> Option<PathBuf>` — `HKCU\Software\Valve\Steam\SteamPath`
    (у ньому `/` → `\`), запасний `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\InstallPath`;
    `scan::default_epic_manifests() -> Option<PathBuf>` — `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`.
  - `scan::scan_library(fs: &dyn LibraryFs, reg: &dyn RegistryApi, src: &Sources, store: &LibraryStore) -> LibraryReport`
    — завжди повертає звіт; кожна помилка джерела/файлу → `LibraryProblem`, решта списку будується.
    Steam-гра показується лише коли `<lib>\steamapps\common\<installdir>` — тека; `has_cover` = файл обкладинки існує.
    Дублікати між усіма джерелами — за `install_dir` (без регістру, без кінцевого `\`), перший виграє
    (порядок Steam, Epic, GOG, Battle.net). Сортування за `name` без регістру. Ручні ігри зі стору додаються з `Launcher::Manual`.
  - `store::LibraryStore` (serde): `{ games: BTreeMap<String, UserGame>, manual: Vec<ManualGame> }`,
    `UserGame { exe: Option<String>, hidden: bool }`, `ManualGame { id: String, name: String, exe: String }`
    (`install_dir` порожній). `LibraryStore::load(path: &Path) -> LibraryStore` (немає/битий файл → порожній, битий — `tracing::warn`),
    `save(&self, path: &Path) -> Result<(), LibraryError>` (запис у тимчасовий файл + перейменування),
    `set_hidden(&mut self, id: &str, hidden: bool)`, `set_exe(&mut self, id: &str, exe: Option<String>)`.
  - `scan::cover_path(src: &Sources, id: &str) -> Option<PathBuf>` — лише для `steam:<appid>`, де appid — самі цифри.

- [ ] **Step 1: Тести** (на `MemFs` + `MemRegistry`): `scan_steam_two_libraries`, `scan_skips_missing_common_dir`,
  `scan_skips_excluded_appid` (228980), `scan_missing_library_drive_is_problem` (бібліотека `E:\SteamLibrary` без файлів →
  `problems.len() == 1`, ігри з D: є), `scan_broken_appmanifest_is_problem`, `scan_epic_and_gog_and_bnet`,
  `scan_dedups_by_install_dir` (та сама тека зі Steam і GOG → один запис зі Steam), `scan_applies_store`
  (hidden/exe зі стору), `scan_no_launchers_is_empty_without_problems`, `cover_path_rejects_non_numeric_appid`
  (`steam:..\x` → `None`), `store_survives_rescan` (save → load → скан без цієї гри → `load` знову містить її `exe`),
  `store_broken_file_is_empty`.
- [ ] **Step 2:** FAIL. **Step 3:** реалізувати. **Step 4:** PASS.
- [ ] **Step 5:** fmt + clippy; зупинка: коміт `feat(gamemode): game library scan and user state`.

### Task 5: Вибір процесу гри

**Files:**
- Create: `crates/gamemode/src/library/detect.rs`
- Test: у файлі

**Interfaces:**
- Produces:
  - `RunningProcess { pid: u32, name: String, path: Option<String>, memory_mib: u64 }`.
  - `inside_dir(path: &str, dir: &str) -> bool` — нормалізація: `/` → `\`, нижній регістр, без кінцевого `\`;
    `false`, якщо `dir` порожній або будь-який шлях містить компонент `..`; збіг лише з роздільником (`dir\`).
    Без звертань до диска.
  - `pick_game<'a>(install_dir: &str, foreground_pid: Option<u32>, procs: &'a [RunningProcess]) -> Option<&'a RunningProcess>` —
    процес на передньому плані, що лежить у теці гри **і** має найбільший `memory_mib` серед процесів цієї теки (рівність — ок).
  - `Detector` (Default): `feed(&mut self, picked: Option<&str>) -> Option<String>` — повертає ім'я exe, коли той самий exe
    (без регістру) обрано `STABLE_POLLS` разів поспіль; `None` або інший exe скидає лічильник. `pub const STABLE_POLLS: u32 = 3;`

- [ ] **Step 1: Тести** — `inside_dir_nested` (`F:\Games\Foo\bin\foo.exe` у `F:\Games\Foo`), `inside_dir_rejects_prefix_sibling`
  (`F:\Games\FooBar\x.exe` не в `F:\Games\Foo`), `inside_dir_mixed_separators_and_case`, `inside_dir_trailing_separator`,
  `inside_dir_rejects_dotdot` (`F:\Games\Foo\..\Bar\x.exe`), `inside_dir_empty_dir_is_false`,
  `pick_foreground_heaviest` (RDR2.exe 6000 МіБ на передньому плані → він), `pick_ignores_light_foreground_launcher`
  (`launcher_epic.exe` 150 МіБ на передньому плані, `GenshinImpact.exe` 4000 МіБ у фоні → `None`),
  `pick_none_without_foreground`, `pick_skips_process_without_path`, `pick_ignores_other_dirs`,
  `detector_needs_three_same_polls`, `detector_resets_on_change`, `detector_case_insensitive`.
- [ ] **Step 2:** FAIL. **Step 3:** реалізувати. **Step 4:** PASS.
- [ ] **Step 5:** fmt + clippy; зупинка: коміт `feat(gamemode): detect game process by install folder`.

> Відхилення від спеки (рішення асистента): визначення процесу — у програмі **перед** стартом запису
> (а не новим станом шлюзу `frametime`), без поля `path` у `ProcessInfo`: шлях і пам'ять беруться з `sysinfo`,
> як у `list_processes`. Запис далі йде наявним шляхом за ім'ям exe з `StartMode::Auto`, який і так чекає,
> поки гра малює кадри на передньому плані. Шлюз і `ProcessApi` не змінюються.

### Task 6: Команди Tauri

**Files:**
- Create: `crates/app/src/commands/library.rs`, `crates/app/src/b64.rs` (кодування base64, ~20 рядків, з тестом)
- Modify: `crates/app/src/commands/mod.rs`, `crates/app/src/lib.rs` (реєстрація команд біля `dashboard_last_game`, ~393),
  `crates/app/src/state.rs` або місце, де оголошено `AppState` (поля нижче), `crates/app/src/bindings.rs` (типи в `export!`),
  `ui/src/lib/api.ts`

**Interfaces:**
- Consumes: завдання 4–5.
- Produces (команди, усі через `blocking(...)`, як `dashboard_*`):
  - `library_scan(refresh: bool) -> LibraryReport` — кеш у `AppState.library: Mutex<Option<LibraryReport>>`; `refresh` або порожній кеш → скан.
    Стор: `ui_dir()/library.json`. Реєстр: той самий тип, що використовує програма для `HKCU` інтерактивного користувача.
  - `library_set_hidden(id: String, hidden: bool) -> ()` — стор + оновити кеш.
  - `library_forget_exe(id: String) -> ()` — `set_exe(id, None)`.
  - `library_cover(id: String) -> Option<String>` — `cover_path` → `read_bytes(.., 2 MiB)` → `"data:image/jpeg;base64,…"`;
    будь-яка помилка → `None` (CSP `img-src 'self' data:` уже дозволяє, `tauri.conf.json` не змінюється).
  - `library_detect(id: String) -> DetectState` (ts): `DetectState = Waiting | Found { exe: String } | NoFolder`.
    Знімок `sysinfo` з `with_exe(UpdateKind::OnlyIfNotSet).with_memory()` у тому ж `AppState.processes`, що `list_processes`;
    `optimax_core::input::foreground_process_id()`; `pick_game` → `Detector` у `AppState.detect: Mutex<Option<(String, Detector)>>`
    (новий `id` — новий детектор). `Found` → `set_exe` у сторі + кеш + `tracing::info!` («гру визначено»).
    Гра без `install_dir` (ручна) → `NoFolder`.
  - `library_add_manual(name: String, exe: String) -> GameEntry` — `Target::ProcessName(exe).validate()` перед збереженням; id `manual:<exe у нижньому регістрі>`.
  - `api.ts`: `libraryScan`, `librarySetHidden`, `libraryForgetExe`, `libraryCover`, `libraryDetect`, `libraryAddManual`.

- [ ] **Step 1: Тест** `b64::tests::rfc4648_vectors` (`""`, `"f"`→`"Zg=="`, `"fo"`→`"Zm8="`, `"foo"`→`"Zm9v"`, `"foobar"`→`"Zm9vYmFy"`) — FAIL → реалізувати → PASS.
- [ ] **Step 2:** команди й реєстрація; `cargo test -p optimax-app` (генерує `ui/src/lib/bindings/{Launcher,GameEntry,LibraryProblem,LibraryReport,DetectState}.ts`).
- [ ] **Step 3:** `cargo check --workspace`, clippy; `cd ui && npm run check`.
- [ ] **Step 4:** зупинка: коміт `feat(app): game library commands`.

### Task 7: `GamePicker` і вбудовування в «Діагностику» та «Ігри»

**Files:**
- Create: `ui/src/components/GamePicker.svelte`
- Modify: `ui/src/screens/Diagnosis.svelte` (рядки ~187 і ~195 — `ProcessPicker` → `GamePicker`; старт запису),
  `ui/src/screens/Games.svelte` (додавання профілю), `crates/core/locales/uk.json`, `ui/src/dev/Gallery.svelte` (стани компонента),
  `ui/tests/e2e/fixtures.ts` + новий `ui/tests/e2e/game-library.spec.ts`

**Interfaces:**
- Consumes: `api.library*` із завдання 6; наявний `ProcessPicker` (вкладка «Запущені / Вручну»).
- Produces: `GamePicker` props: `{ value?: string ($bindable, ім'я exe — як у ProcessPicker), game?: GameEntry | null ($bindable), label: string }`.
  Вибір гри з відомим `exe` → `value = exe`; без `exe` → `value = ""`, `game` заповнено.

Поведінка (рішення, які компонент не може вивести сам):
- Сегментований перемикач `Мої ігри | Запущені процеси` (`Segmented`); друга вкладка — наявний `ProcessPicker` без змін.
- Сітка карток: обкладинка (`libraryCover`, лінива — лише видимі картки, кеш у межах сесії) або плитка з першою літерою назви;
  назва; бейдж лаунчера **текстом**; рядок «Процес: RDR2.exe» або «Процес визначиться під час першого запуску».
  Меню картки: «Приховати», для визначеного exe — «Скинути процес».
- Пошук за назвою; «Показати приховані (N)»; «Оновити список» (`libraryScan(true)`); «Додати гру вручну» (назва + exe).
- Стани: завантаження / порожньо («Лаунчерів не знайдено — вибери процес у вкладці «Запущені процеси»») / помилка /
  `problems` — тихий рядок під сіткою з переліком.
- Під час запису, A/B чи стабільності — без опитувань (як `ProcessPicker`).
- «Діагностика», гра без `exe`: кнопка старту → фаза «Запусти гру як звичайно — чекаю, поки вона з'явиться на екрані»
  (опитування `libraryDetect` раз на 2 с, кнопка «Скасувати»); `Found` → рядок у журналі «Гру визначено: X» і одразу
  наявний `api.diagStart(exe, seconds, scene)`.
- «Ігри», додавання профілю для гри без `exe` — та сама фаза очікування, далі наявний редактор профілю з цим exe.

- [ ] **Step 1:** `game-library.spec.ts` (Playwright на моках, як `diagnosis-premium.spec.ts`): список рендериться з назвами й бейджами;
  пошук фільтрує; «Приховати» ховає й показується в «Показати приховані»; порожній стан; гра без exe → фаза очікування →
  мок `Found` → виклик `diagStart` з цим exe. Запустити `npm run test:ui -- game-library` — FAIL.
- [ ] **Step 2:** реалізувати компонент, ключі i18n, вбудовування; галерея — стани «список / порожньо / помилка / очікування».
- [ ] **Step 3:** `npm run check`, `npm run check:i18n`, `npm run check:e2e`, `npm run test:ui` — PASS (усі наявні e2e теж).
- [ ] **Step 4:** зупинка: коміт `feat(ui): pick a game from the library`.

### Task 8: Документація й підсумкова перевірка

**Files:**
- Modify: `CHANGELOG.md` (`[Unreleased]` → «Додано»), `docs/HANDOFF.md` (новий розділ 2w: що зроблено, відхилення від спеки, що не перевірено)

- [ ] **Step 1:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
  `cd ui && npm run check && npm run check:i18n && npm run test:ui` — усе PASS; вивід — у звіт.
- [ ] **Step 2:** записати CHANGELOG і HANDOFF; у звіті розділити «компілюється / покрито тестами / перевірено на Windows».
- [ ] **Step 3:** чекліст користувачу (зі спеки): ігри з D: і E: (Steam), RDR2 / Witch It / Genshin (Epic), Kerbal Space Program (GOG),
  Call of Duty (Battle.net); немає Unreal Engine, плагінів, Battle.net-лаунчера, Steamworks Redistributables; обкладинки Steam видно;
  запис RDR2 з бібліотеки визначає `RDR2.exe`, Genshin — `GenshinImpact.exe`, а не лаунчер; відключений диск E: не ламає список;
  програма зі службою й без неї дає однаковий список.
- [ ] **Step 4:** зупинка: коміт `docs: game library`.
