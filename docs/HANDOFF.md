# Передача роботи між сесіями

Файл для нового агента (іншої сесії Claude Code), щоб продовжити роботу без попереднього
контексту. Оновлюй його наприкінці кожної сесії. Обов'язкові правила — у `CLAUDE.md`.

Останнє оновлення: 2026-10-24.

---

## 1. Як працювати з користувачем

- Спілкування українською, на «ти», як із другом. Коротко й по суті.
- **Git робить лише користувач.** Ніколи не виконувати `git add/commit/push/merge/checkout/reset`
  чи інші команди, що змінюють стан репозиторію. Можна лише читати (`status`, `log`, `diff`).
  Коли потрібна гілка / коміт / злиття — запропонувати команди, користувач виконає сам.
- Нова залежність або архітектурне рішення — спершу питати (`CLAUDE.md`, розділ 13).
- Після кожного кроку / етапу — зупинитись, дати звіт і чекліст ручної перевірки, чекати.
- Чесно розділяти: «компілюється» / «покрито тестами» / «перевірено на реальному Windows».
- Жодного плацебо: без виміряного ефекту — без рекомендації.

## 2. Стан проєкту

Усі шість етапів із `CLAUDE.md` завершені й злиті в `main`:
1. core / metrics / audit; 2. frametime (вбудований PresentMon) + diagnosis;
3. tweaks з журналом і відкатом + A/B; 4. gamemode; 5. Tauri 2 UI (Svelte 5);
6. upscale (WGC + D3D11, фільтри nearest/bilinear/Lanczos/FSR 1/NIS, власний курсор,
   Ctrl+Shift+F10, виміри GPU-часу й доданої затримки, екран «Апскейл», поле в профілі гри,
   вердикти діагностики «апскейл допоможе / не допоможе»).

Після етапів: старт запису «з гри» (Ctrl+Shift+F9 / авто), повідомлення поверх гри
(`optimax_core::toast`), центрування колонки вмісту на широких екранах.

## 2a. Заставка запуску (на `main`, не закомічено)

- `ui/src/lib/splash.ts` — окремий canvas поверх усього; стартує в `main.ts` до запитів бекенду,
  `ready()` після `mount`, `cancel()` при помилці запуску. Ціль польоту — `[data-splash-mark]`
  (знак у `Sidebar.svelte`). Класи на `<html>`: `splashing` (контент прихований, App.svelte),
  `splash-mark` (знак прихований до прильоту точок).
- Дозвіл показу береться з `localStorage["optimax.splash"]` (налаштування ще не прочитані);
  оновлюється `$effect` у `App.svelte` з `ui.settings.animation`/`economy`.
- Час кадру пишеться в консоль DevTools: `[splash] N точок, K кадрів, X мс/кадр`. **Не виміряно**
  на реальному запуску — потрібна перевірка користувачем.

## 3. Поточна робота: гілка `feature/cleaning` (модуль «Очищення», CLAUDE.md розд. 11)

Стан: реалізовано, **не закомічено** (комітить користувач). Перевірки зелені (fmt, clippy
`-D warnings`, 258 тестів, svelte-check, check:i18n).

- Крейт `crates/cleanup` (`optimax-cleanup`), без залежності від tweaks/diagnosis/frametime:
  `path` (нормалізація Windows-шляхів рядками, відмова на `..`/ADS/UNC), `pattern`
  (`%VAR%` + `*` як цілий компонент), `guard` (жорсткий чорний список: заборонені зони +
  контейнери), `rules` (TOML-база, валідація → пониження до ⚪), `fs` (`FsApi`, `StdFs`,
  `MemFs` з junction/8.3/зайнятими файлами), `scan` (корені правил + паралельний обхід диска
  на `std::thread`), `plan` (сухий прогін і виконання з повторною перевіркою кожного файлу),
  `quarantine`, `journal`, `snapshot` («Що виросло»), `system` (DISM, powercfg, Провідник).
- База: `crates/cleanup/rules/*.toml`, 21 правило, у кожному `source` і `verified_on`.
  Шляхи звірено на ПК розробника (Windows 11 Home 22631) і з документацією / обробниками
  `VolumeCaches` у реєстрі. **Не додано** (немає офіційного джерела й тек на ПК розробника):
  розпаковані інсталятори NVIDIA `C:\NVIDIA\DisplayDriver` і AMD `C:\AMD`, Delivery
  Optimization, Steam htmlcache, `target/`/`node_modules`.
- Застосунок: `crates/app/src/commands/cleanup.rs`, події `cleanup-progress`,
  `cleanup-scan-done`, `cleanup-exec-progress`, `cleanup-exec-done`, `cleanup-error`;
  налаштування `Settings.cleanup` (термін і диск карантину, автовидалення після терміну).
  Дані: `%LOCALAPPDATA%\OptimaX\{cleanup-journal.json, quarantine.json, snapshots-<диск>.json,
  Quarantine\}`.
- UI: `ui/src/screens/Cleanup.svelte` + `components/Cleanup*.svelte`, `VerdictBadge.svelte`;
  глобальний стан `cl` у `state.svelte.ts`. TS-типи очищення з конфліктними іменами
  перейменовано: `CleanupVerdict`, `CleanupCategory`, `CleanupOutcome`.
- Наживо перевірено (без адміна, лише читання): скан за правилами ~8 с, повний скан C:
  (80 ГБ) ~14 с release / ~27 с debug, сухий прогін, усі вкладки екрана. **Не перевірено**:
  справжнє видалення / карантин / відновлення, DISM, powercfg, точка відновлення, зупинка
  wuauserv — потребують прав адміністратора й згоди користувача.
- Приклад: `cargo run --release -p optimax-cleanup --example scan -- C:\ [full]` (лише читання).
- Пришвидшення скану (2026-10-25): корінь правила обходять кілька потоків (`walk_root_with`,
  результат не залежить від кількості потоків — тест); прогрес показує корінь, який
  обходиться зараз, і оновлюється кожні 200 мс; 🔴-корені (WinSxS, Installer, DriverStore)
  обходяться останніми, а подія `cleanup-scan-partial` показує знайдене до кінця скану.
  Заміри на ПК розробника (теплий кеш, 3 прогони, без адміна): скан за правилами 7,4–8,3 с
  (1 потік) → 2,2–3,5 с (4 потоки), проміжний звіт ~0,5 с. Холодний кеш не заміряно.
- Рядок із правилом, під умови якого зараз нічого не підпадає (`Item.nothing_eligible`,
  напр. Temp, де всі файли відкривались <7 днів тому): неактивний чекбокс і пояснення.
  Відомо: розмір WinSxS завищений (жорсткі посилання рахуються як окремі файли).
- «Невідоме» (2026-10-25, варіант A): `ScanReport.unknown` — лише ⚪, зони чорного списку —
  окремо в `ScanReport.protected_zones` (група «Відоме, не чіпати», згорнута); у кожного рядка
  повного скану `small_dirs` і `top_subdirs` (до 8 найбільших дрібних підтек, «Показати в
  Провіднику» дозволено й для них). **Не зроблено (варіант B, чекає рішення):** правила з
  джерелами для NativeImages, SystemApps, Package Cache, VS Packages, `.nuget\packages`,
  NVIDIA NGX / UpdateFramework, Spotify, кешів JetBrains — без перевірки не додавати.

## 3a. Попередня гілка `feature/laptop-network` (злито в `main`)

Мета — зробити повністю робочими екрани **«Мережа»** і **«Ноутбук»** (зараз решта
екранів-заглушок: Програми, Автозавантаження, Вигляд — не чіпати без запиту).

### 3.1 Мережа — ГОТОВО (закомічено: `feat(network): …`)

- Крейт `crates/network` (`optimax-network`, додано в структуру `CLAUDE.md`):
  - `model.rs` — адаптер, тип з'єднання (Ethernet / Wi-Fi / Virtual за прапорцем
    `HardwareInterface` з `GetIfEntry2`, бо VPN «прикидаються» Ethernet), Wi-Fi (діапазон і
    канал за частотою BSS), `PingStats` (avg / median / p95 / джитер / втрати).
  - `rules.rs` — висновки з доказами: де проблема (вдома / за межами дому), Wi-Fi 2,4 ГГц,
    слабкий сигнал, VPN у маршруті, кабель на 100 Мбіт/с, велика затримка.
  - `runner.rs` — паралельний пінг кількох цілей через трейт `EchoApi` (тести з імітацією).
  - `win/` — IP Helper (`GetAdaptersAddresses`, `GetBestInterfaceEx` — без пакетів),
    WLAN API, `IcmpSendEcho2` з власним заміром часу (поле RTT у відповіді — цілі мс).
  - Приклад: `cargo run --release -p optimax-network --example network -- [вузол] [секунди]`.
- Застосунок: `crates/app/src/commands/network.rs` (`net_info`, `net_test_start`,
  `net_test_cancel`; події `net-progress`, `net-done`, `net-error`).
- UI: `ui/src/screens/Network.svelte`.
- Рішення користувача: пінг лише за кнопкою, до роутера + вузла за вибором (за замовчуванням
  DNS, якщо це не сам роутер; у користувача DNS = роутер, тож поле порожнє з підказкою).
  Мережевих «твіків» (Nagle тощо) **не пропонувати** — на екрані є пояснення чому.
- Перевірено на ПК користувача: адаптери визначено правильно (Radmin VPN і Tailscale —
  віртуальні), замір до роутера 150/150, ~0,5 мс, «Мережа стабільна». Пінги до зовнішніх
  вузлів наживо не слались (рішення користувача — нічого без його відома).

### 3.2 Ноутбук — ГОТОВО (реалізовано, потребує перевірки на реальному ноутбуці)

#### Що реалізовано

- **`crates/core/src/power.rs`** — overlay GUIDs (економія / збалансований / краща / найкраща
  продуктивність) + нові методи `PowerApi`: `get_effective_overlay`, `set_active_overlay`.
- **`crates/core/src/power/windows.rs`** — `WinPower` реалізує overlay-методи через
  `GetProcAddress("powrprof.dll", "PowerGetEffectiveOverlayScheme"/"PowerSetActiveOverlayScheme")`
  ; якщо експорту немає — повертає `None`, не падає.
- **`crates/audit/`** — типи `BatteryInfo`, `PowerSource`; читання батареї через
  `IOCTL_BATTERY_QUERY_*` (SetupAPI); правила `check_battery` (знос, живлення від батареї під
  час гри тощо).
- **`crates/tweaks/`** — `GpuPref` (enum: Unset / PowerSaving / HighPerf), `TweakId::PowerMode`
  і `TweakId::GpuPreference`, нові варіанти `Desired` та `SettingValue`, читання/запис/план у
  `engine.rs`.
  - GPU preference пише реєстровий ключ
    `HKCU\Software\Microsoft\DirectX\UserGpuPreferences` (ім'я = повний шлях exe).
  - Power mode використовує `SettingTarget::PowerMode` / `SettingValue::PowerMode(GUID)`.
- **`crates/app/src/commands/laptop.rs`** — `LaptopView` (батарея + overlay-режим + GPU-преф),
  команда `laptop_info`.
- **TypeScript bindings** — `BatteryInfo.ts`, `PowerSource.ts`, `GpuPref.ts`, `LaptopView.ts`,
  `UiDesired.ts` перегенеровано.
- **`ui/src/screens/Laptop.svelte`** — екран з картками: стан батареї (заряд, знос, цикли),
  segmented control режиму живлення (схований, якщо API недоступне), GPU preference для
  вибраної гри.
- **`ui/src/App.svelte`** — маршрут `laptop` підключений.
- **`ui/src/components/Sidebar.svelte`** — `"laptop"` додано до `ready` Set.
- `cargo check --workspace` проходить чисто.

#### Потребує перевірки на реальному ноутбуці (десктоп не підходить)

- Батарея: чи читаються IOCTL-дані (`design_mwh`, `cycle_count`, `wear_percent`).
- Overlay mode: чи `PowerGetEffectiveOverlayScheme` доступний і повертає правильний GUID.
- GPU preference: чи реєстровий ключ `UserGpuPreferences` пишеться і читається коректно.
- Перевірити стан «це не ноутбук» (батарейний блок відсутній) — UI повинен коректно
  відображатись на десктопі (картка батареї прихована / показує «не виявлено»).

#### Погоджені рішення (повторно не питати)

- Окремого крейту для ноутбука **немає** — стан і знахідки в `crates/audit`, зміни в
  `crates/tweaks`.
- GPU-preference береться лише через вибір exe з запущених процесів або файлового діалогу —
  повний шлях до exe є ключем реєстру.
- В UI є позначка «недокументоване API» біля overlay-режиму.
- Мережевих «твіків» (Nagle тощо) **не пропонувати**.

### 3.3 Що залишилось у гілці `feature/laptop-network`

- **Тести** для `check_battery` (знос, правила), `engine` PowerMode plan → apply → rollback.
- `README.md` — заглушка користувача (UTF-16, «GateX»); не чіпати без запиту.
- Запропонувати користувачу виняток для шрифтів OFL у `CLAUDE.md` (Inter, JetBrains Mono
  вбудовані з його згоди) — це його файл, лише запропонувати.
- Генерація кадрів — лише експеримент за окремим запитом (`CLAUDE.md`, розділ 11).

## 4. Середовище користувача

Windows 11 Pro, Ryzen 7 5800X, RTX 4070 SUPER; основний монітор 2560×1440 @ 200 Гц,
другий 1920×1080 ліворуч (координати від −1920). Ethernet 1 Гбіт/с, роутер 192.168.1.1,
віртуальні адаптери Radmin VPN і Tailscale. Тестова гра — Geometry Dash (`GeometryDash.exe`,
у вікні 1280×720 для апскейлу). Застосунок вимагає прав адміністратора.

## 5. Технічні пастки (уже траплялись)

- **C: майже повний** (~3–4 ГБ вільно): окремі `CARGO_TARGET_DIR` для збірок класти на D:
  (напр. `D:/Projects/PCOptimize/OptimaX/target/dev-invoker`), не в тимчасову теку на C:.

- **Dev-збірка без адміна:** `OPTIMAX_DEV_AS_INVOKER=1` діє **під час збірки** (build.rs),
  не запуску: `OPTIMAX_DEV_AS_INVOKER=1 cargo build -p optimax-app`. Debug-збірка бере UI
  з Vite (`http://localhost:1420`, `cd ui && npx vite --port 1420`).
- Якщо користувач тримає запущеним свій `target\debug\optimax.exe`, файл заблоковано —
  збирати в окрему теку: `CARGO_TARGET_DIR=<scratchpad>/target-app`. Чужі процеси не вбивати.
- Автоматизація UI з PowerShell: `FindWindow($null, …)` не працює (PowerShell передає `""`) —
  брати `(Get-Process).MainWindowHandle`; перший клік у WebView2 лише забирає фокус —
  спершу клік у порожнє місце. Знімки екрана — лише вікна застосунку / свого оверлею.
- TS-типи генерує тест `cargo test -p optimax-app` (`crates/app/src/bindings.rs`) у
  `ui/src/lib/bindings/`. Однакові імена типів з різних крейтів затирають файл — тоді
  `#[cfg_attr(feature = "ts", ts(rename = "…"))]` (так зроблено `TimingSummary`).
- `ui/scripts/check-i18n.mjs` шукає ключі за префіксами; динамічні ключі (шаблонні рядки)
  треба дописувати в нього вручну. Тексти — лише в `crates/core/locales/uk.json`.
- Svelte 5: `{#snippet}` всередині тегу компонента стає його пропом — оголошувати поза ним.
- Svelte 5: `{@const r = x}` — похідне значення. Якщо в обробнику спершу `x = null`, а потім
  читати `r`, там уже `null` (тиха TypeError, дія не виконується). Брати потрібне з `r` до
  обнулення. Саме так «Продовжити без точки відновлення» в Очищенні нічого не робило.
- Модалки (`Modal.svelte`) займають лише область вмісту (`left: --sidebar-width`): сайдбар
  накладається поверх, тож вікно на весь екран ховалося під нього при масштабі 125%.
- Автоматизація UI: якщо на передньому плані вікно з правами адміністратора (OptimaX від
  адміна, адмінський PowerShell), Windows блокує штучні кліки зі звичайного процесу. Скрипт
  кліків прив'язувати до PID свого тестового процесу, не до першого вікна `optimax`.
- Апскейл: системний курсор, схований через `MagShowSystemCursor`, WGC у кадр не малює —
  тому курсор малюється власним шаром; черга показу обмежена 1 кадром + пропуск кадру, що
  не встигає на свій V-Sync (інакше стійке відставання на кадр).
- Тестові утиліти попередньої сесії (вікно на 240 к/с `fastwin`, PowerShell-скрипти тестових
  вікон) лежали в тимчасовій теці й на інший пристрій не переносяться; за потреби відтворити.

## 6. Перевірки перед звітом

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # зараз 258 тестів
cd ui && npm run check && npm run check:i18n
```
