# Передача роботи між сесіями

Файл для нового агента (іншої сесії Claude Code), щоб продовжити роботу без попереднього
контексту. Оновлюй його наприкінці кожної сесії. Обов'язкові правила — у `CLAUDE.md`.

Останнє оновлення: 2026-11-01 (після релізу 0.1.5).

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

Релізи: остання версія **0.1.5** (2026-11-01, тег `v0.1.5`). Реліз — тег `vX.Y.Z` → GitHub
Actions (`.github/workflows/release.yml`) збирає zip, інсталятор NSIS і `latest.json`;
процедура — `docs/RELEASE.md`. Версію піднімати в `Cargo.toml`, `crates/app/tauri.conf.json`,
`ui/package.json`, `ui/package-lock.json` (+ `Cargo.lock` через `cargo check`), CHANGELOG
`[Unreleased]` → `[X.Y.Z] - дата`. Після пушу тегу перевіряти: run success, `latest.json`
з новою версією, ідентифікатор ключа підпису `54e47109ac37bdcc`, інсталятор віддає 200.

## 2h. Сесія 2026-11-01 (усе закомічено, у 0.1.5, крім останнього пункту)

- **Плашка оновлення** `ui/src/components/UpdateBanner.svelte` (закривається на версію,
  `localStorage`). Перевірка при старті лишається вимкненою за замовчуванням (CLAUDE.md 5.7).
- **Радар мікрофризів:** ознака DPC за PDH (`optimax_core::perf::DpcCounter`, найзайнятіше
  ядро), зведення причин біля ривків (`chart::CauseCount`), «тло» (ознака біля піків не
  частіше, ніж зазвичай, `ABOVE_USUAL = 2`), вердикт `stutter.partial`, правила
  `vsync_judder` і `periodic_spikes`.
- **Тротлінг GPU:** вплив за виміряною втратою частоти, оберти вентиляторів, Power Limit понад
  заводський (NVML).
- **ETW-трасування драйверів (DPC рівень 2):** власна обгортка `crates/core/src/etw.rs` +
  `etw/windows.rs` (системна сесія `OptimaX DPC Trace`, `EVENT_TRACE_SYSTEM_LOGGER_MODE`,
  QPC-час, сирий розбір подій `PerfInfo` 50/66/67/68/69, модулі через `EnumDeviceDrivers`).
  Запускається в `frametime::record_with`, результат — `Recording::dpc_trace` →
  `Timeline::dpc`. Правила: виконання ≥ 0,5 мс за 100 мс до ривка = ознака DPC з іменем
  драйвера; `diagnosis/src/drivers.rs` — тип пристрою за іменем і порада. Перевірено на ПК
  користувача: ~5 тис. подій/с у спокої, ~15 тис./с у RE9, 0 втрачених. Тест сесії —
  `cargo test -p optimax-core etw -- --ignored --nocapture` **у PowerShell від адміністратора**.
  Переривання відеокарти Windows проводить через `dxgkrnl.sys` (тому він — «відеокарта»).
- **Ліцензія і SignPath:** `LICENSE-MIT` / `LICENSE-APACHE`, розділ «Code signing policy» у
  README. Заявку на SignPath Foundation користувач подає сам; підключення підпису до CI —
  лише після схвалення. Ризики: молодий репозиторій (репутація), `webpki-roots` під
  CDLA-Permissive-2.0 (не OSI).
- **Не закомічено / не в релізі:** прогрес завантаження у вікні оновлення
  (`ui/src/screens/Settings.svelte`) — перевірити наживо можна лише на оновленні 0.1.5 → 0.1.6.

## 2i. Перевірка RAM — пункт (4) (закомічено `c231150`)

- `smbios.rs`: Type 17 тепер читає Bank Locator (0x11) → `MemoryModule.bank`; усі структури
  Type 17 (з порожніми слотами) рахуються в `FirmwareInfo.memory_slots`.
- `audit/src/memlayout.rs` — чисті функції: `channel_of(bank, locator)` (лише явне `CHANNEL x`,
  контролер — `ControllerN` або сокет `PN`; `DIMM_A1`, `DIMM 0` → невідомо, не вгадуємо),
  `check_memory_layout` → `ram.single_module` (High), `ram.single_channel` (High),
  `ram.asymmetric` (Moderate), `ram.multi_channel` / `ram.channels_unknown` (Info), `ram.mixed`
  (Info). Для LPDDR і невідомих типів канальних вердиктів немає.
- Реальні дані ПК користувача (AMD AGESA): `P0 CHANNEL A/B` + `DIMM 1`, 2 × 16 ГБ, 4 слоти →
  «2 канали по 16 ГБ» (перевірено прикладом `cargo run -p optimax-audit --example audit`).
- Схеми підписів Intel (`BANK n` + `ChannelA-DIMM0` / `Controller1-ChannelA-DIMM1`) — лише з
  тестів, на реальному Intel-ПК не перевірено.
- `audit/src/partnum.rs` — `kit_speed_mts(part, type)`: G.Skill (`F4-3600C18…`, `F5-6000J3038…`,
  специфікації gskill.com + SPD на ПК користувача) і Corsair (`CMK32GX4M2Z3200C16`, офіційний
  гайд: «3200 — MT/s з профілем розгону»). Покоління в партномері має збігатися з типом модуля.
  У `check_memory`: усі модулі розпізнано → `ram.below_kit` (High, 90) або `ram.at_kit` (Info);
  інакше стара евристика `ram.jedec_base`. Kingston FURY не додано: декодер kingston.com
  віддає 403 автоматичним запитам, а поле частоти не завжди × 100 (`26` = 2666). Чи пише
  Corsair номер набору в SPD модуля — не перевірено (якщо ні, просто не розпізнається).

## 2j. Тест чутливості до роздільності — пункт (2) (закомічено `f343b05`; перевірено в RE9 з DLSS: +21,4 %)

- Рішення користувача: **2+2 прогони, порядок AABB** (у грі перемикати лише раз), кнопка — у
  результаті діагностики, коли вузьке місце GPU або CPU.
- `diagnosis/src/resolution.rs` — чисті функції: `ResolutionRun::from_recording`, `evaluate`
  (поріг «допоможе» ≥ 10 % і > 2 × розкид повторів; «не допоможе» ≤ 3 %; падіння GPU < 5 в. п.
  при незмінному FPS → `not_applied`; будь-який прогін у ліміті кадрів → `capped`; решта →
  `unclear`), `apply` (вердикт `res.*` одразу після підсумку, `gpu_bound` → 95 % / 40 % з іншим
  `detail`, доказ «Тест роздільності» й `expected_gain` для gpu/cpu_bound та upscale.*).
- `app/src/commands/resolution.rs`: `res_start`, `res_state`, `res_run_next`, `res_cancel`, подія
  `res-progress`; уточнення пишеться в `AppState.last_diagnosis`. Запис прогону винесено в
  `commands::record_game` (спільний з A/B). Діагностика й A/B не стартують під час прогону тесту;
  нова діагностика закриває старий тест.
- UI: `ui/src/components/ResolutionTest.svelte` (кроки 1–4, підказка «знизьте роздільність» перед
  3-м прогоном, підсумок), підключено в `Diagnosis.svelte`.
- Перевірено користувачем у re9.exe (DLSS «Якість» → «Продуктивність»): «допомагає +21,4 %»,
  розкид 6,7 %. Знайдено й виправлено: підсумок `summary.gpu` не отримував нову впевненість.
  Пороги 10 % / 3 % / 5 в. п. — обрані, не відкалібровані.

## 2k. «Щойно лагнуло» — пункт (7) (не закомічено, чекає перевірки в грі)

- Рішення користувача: стеження **вмикається кнопкою** на «Діагностиці» (не автоматично),
  **Ctrl+Shift+F11** зберігає знімок останніх 30 с **у список** (до 10 за сесію, лише в
  пам'яті), стеження йде далі.
- `core/src/etw.rs`: кільцевий режим колектора DPC (`DpcTracer::start_ring(freq, keep_secs)`,
  `window(from, to)` — зріз без зупинки сесії): статистика по секундних кошиках, довгі події
  старші за `keep` відкидаються. `etw/windows.rs`: колектор тепер `Arc<Mutex<Collector>>`
  (зворотний виклик ETW бере замок), `lost_so_far` через `ControlTrace(QUERY)`.
- `core/src/input.rs`: `FunctionKey::F11`. **Гарячі клавіші тепер ще й опитуються**
  (`GetAsyncKeyState` для Ctrl, Shift і F-клавіші раз на 25 мс, захист від подвійного спрацювання
  400 мс): у RE9 `RegisterHotKey` у фокусі гри не спрацьовував (ні F9, ні F11 — перевірено
  прикладом `cargo run -p optimax-core --example hotkey_probe -- f11 40`; на робочому столі
  працюють). Стосується й запису (F9), і апскейлу (F10).
- `frametime/src/watch.rs`: `Ring` (чиста структура: кадри, знімки метрик, відрізки гри у фокусі
  після прогріву 2 с, `slice` → `Recording`), `Watcher` (PresentMon без ліміту гри —
  перезапуск після `MAX_CAPTURE` або збою, 3 швидкі збої поспіль → `PresentMonFailed`; семплер
  200 мс; DPC-кільце 40 с; після натискання чекає `DRAIN` 2,5 с, потім вирізає вікно).
  Менше 5 с гри у фокусі у вікні → `NotEnoughGame`. Події: `Status` щосекунди, `Captured`,
  `CaptureFailed`, `Ended` (`Stopped` / `GameExited` / `PresentMonFailed`).
- `app/src/commands/lag.rs`: `lag_start`, `lag_state`, `lag_capture`, `lag_stop`, `lag_close`;
  події `lag-status`, `lag-update`. Знімок аналізується `diagnose` з тим самим контекстом, що й
  звичайний запис (`diagnosis::context`, `shader_cache_files`). Поки йде стеження, діагностика,
  A/B і тест роздільності не стартують (одна ETW-сесія PresentMon і одна DPC на систему), і
  навпаки.
- UI: `ui/src/components/LagWatch.svelte` під карткою налаштування запису в `Diagnosis.svelte`.
- Навантаження: приклад `cargo run -p optimax-frametime --example watch -- game.exe 120`
  (адміністратор) друкує CPU OptimaX і PresentMon раз на 10 с. `sysinfo` додано лише як
  dev-залежність `optimax-frametime` для цього прикладу. Виміряно користувачем (debug, RE9,
  80 с): OptimaX ≈ 0,4 % CPU / 49 МіБ, PresentMon ≈ 0,1 % / 44 МіБ, пам'ять не росте.
- Перевірено користувачем прикладом `watch` у RE9 (адміністратор): F11 у грі → знімок 4105 кадрів,
  30,0 с гри, 160 знімків метрик, 392 703 події DPC (8606 довгих); у середньому OptimaX 0,41 %,
  PresentMon 0,10 % CPU. **Екран у програмі (`LagWatch.svelte`) ще не перевірено.**

## 2l. Оверлей у грі — пункт (6), етапи 1 (`1c5e6a4`) і 2

- Рішення користувача: малювати **GDI** (як повідомлення, не wgpu апскейлу); **авто, як
  Afterburner** — гра у фокусі, без вибору exe; **два етапи**. Вміст (погоджено): `137 FPS 1% low
  92` / `7.3 мс` / `CPU 41 % ядро 88 %` / `GPU 97 %`; етап 2 — «● Запис» під час запису/A/B/
  стеження. Лівий верхній кут, Ctrl+Shift+F12 — сховати/показати, налаштування вимкнене за
  замовчуванням.
- `core/src/overlay.rs`: вікно (WS_EX_LAYERED|TRANSPARENT|NOACTIVATE|TOPMOST, `UpdateLayeredWindow`,
  Consolas, токени кольорів, альфа 225), команди через канал + `WM_APP`. `FunctionKey::F12`.
- `frametime/src/live.rs`: `FrameWindow` (чиста: кадри процесу у фокусі, FPS і час кадру за 2 с
  від найновішого кадру — PresentMon запізнюється ~1 с; 1% low за 10 с від 100 кадрів),
  `LiveMonitor` (PresentMon **по всіх процесах** в окремій сесії `OptimaX Live` — `live_args`,
  `spawn_live`, `stop_live_session`, тож не конфліктує із записом/стеженням; семплер 500 мс без
  переліку процесів; перезапуск PresentMon; кадрів > 2 с немає → `None`, оверлей ховається).
- `app/src/commands/overlay.rs`: `Controller` (потік: вікно + F12 + `LiveMonitor`, оновлення
  2 р/с), `apply(app, enabled)` з `set_settings` і при старті, `stop` при виході; `lines()` —
  тексти `overlay.*` з каталогу. `Settings.overlay`. Тумблер у «Налаштуваннях».
- Замір впливу: `cargo run -p optimax-frametime --example overlay_cost -- 20` (адміністратор,
  4 фази без/з/без/з оверлеєм, різниця проти розкиду однакових фаз). **Виміряно в RE9
  (2026-11-02):** без 159.0 FPS, з 158.6 FPS — −0.24 % при розкиді однакових фаз 0.36 %, тобто
  впливу не видно. 1% low: 123.9 / 114.6 / 124.1 / 123.4 — провал лише у фазі 2, у фазі 4 його
  немає, тож це разовий пік, а не оверлей. Вікно оверлею під час заміру показувалося. Перемикач у
  програмі, F12 і одночасний запис ще не перевірені.
- Відомі межі: не видно в ексклюзивному повноекранному режимі; показується й над браузером чи
  іншою програмою, що малює кадри у фокусі (як RTSS); GPU — лише NVML.
- **Етап 2 (зроблено, не закомічено, не перевірено на Windows):** перший рядок — позначка.
  `AppState.record_progress` заповнює `commands::ProgressMark` (guard, знімається й при помилці)
  у `diag_start` і `record_game` (A/B, тест роздільності). `overlay::mark()` (чиста):
  WaitingForGame/WaitingForHotkey/Warmup/Paused — сірим, `● Запис {done} / {total} с` і
  `● Стеження · Ctrl+Shift+F11` (`lag::is_watching`) — `--danger`. Тексти `overlay.mark.*`.
  Дані із запису замість власної сесії **не** робили: замір показав, що окрема сесія FPS не
  знижує, а спільне джерело ускладнило б запис.

## 3. Наступні завдання (обрав користувач 2026-11-01)

Робити **по одному**, після кожного — звіт, чекліст і зупинка. Нові залежності — спершу питати.
Порядок, який раджу (від малого до великого): 4 → 2 → 7 → 10.

- **(4) Перевірка RAM:** одноканальний режим, змішані модулі, реальна частота проти
  номінальної. Джерела: WMI `Win32_PhysicalMemory` (`BankLabel`, `DeviceLocator`, `Capacity`,
  `Speed`, `ConfiguredClockSpeed`, `PartNumber`), за потреби SMBIOS. Поруч уже є перевірка
  XMP в `crates/audit` — розширювати там. Канали за `DeviceLocator`/`BankLabel` визначаються
  неоднозначно (різні BIOS пишуть по-різному) — перевірити на реальних даних і не вигадувати
  вердикт, якщо не впевнені.
- **(2) Тест чутливості до роздільності:** CLAUDE.md 9 вимагає для GPU-bound перевіряти
  залежність FPS від роздільності; зараз вердикт пише «не перевірялось» і обмежує
  впевненість. Два короткі записи (рідна роздільність і знижена — render scale / DLSS
  Performance, змінює користувач у грі) → «знизиш роздільність — +N % FPS» або «не допоможе».
  Рушій A/B (`crates/app/src/commands/ab.rs`) можна взяти за основу.
- **(7) «Щойно лагнуло»:** кільцевий буфер останніх ~30 с (PresentMon + метрики + DPC) під час
  гри; гаряча клавіша (не Ctrl+Shift+F9 — запис, не Ctrl+Shift+F10 — апскейл) зберігає й
  аналізує буфер. Перевірити навантаження довгої роботи PresentMon і ETW у фоні.
- **(10) Дашборд замість «Аналізу»:** картка «Стан ПК» з підсумками (вузьке місце останньої
  гри, проблеми конфігурації, що можна звільнити на диску, оновлення). Лише показ уже
  обчисленого бекендом, без нової логіки у фронтенді (CLAUDE.md 10.8).

Інші ідеї зі списку (на потім): «до / після» для будь-якої дії користувача, аналіз потоків
гри через ETW `CSwitch` (P/E-ядра, CCD з V-Cache), перевірка накопичувача гри, оверлей,
стрес-тест стабільності, звіт для форуму, спрощення навігації, світла тема й акценти,
інтерактивний графік frametime, онбординг, winget/Scoop, AMD/Intel GPU через лічильники
`\GPU Engine(*)` (зараз метрики GPU лише NVIDIA — найбільша прогалина ядра).

## 2a. Заставка запуску (на `main`, закомічено `24065b0`)

- `ui/src/lib/splash.ts` — окремий canvas поверх усього; стартує в `main.ts` до запитів бекенду,
  `ready()` після `mount`, `cancel()` при помилці запуску. Ціль польоту — `[data-splash-mark]`
  (знак у `Sidebar.svelte`). Класи на `<html>`: `splashing` (контент прихований, App.svelte),
  `splash-mark` (знак прихований до прильоту точок).
- Дозвіл показу береться з `localStorage["optimax.splash"]` (налаштування ще не прочитані);
  оновлюється `$effect` у `App.svelte` з `ui.settings.animation`/`economy`.
- Час кадру пишеться в консоль DevTools: `[splash] N точок, K кадрів, X мс/кадр`. **Не виміряно**
  на реальному запуску — потрібна перевірка користувачем.

## 2b. Вердикт режиму показу (PresentMode) (у релізах)

- `optimax_frametime::PresentMode` — 7 рядків дослівно з `PresentModeToString`
  (PresentMon v2.6.0, `PresentMon/CsvOutput.cpp`), решта → `Other`. `is_composed()`:
  `Composed: *` → так; `Hardware*` (зокрема `Hardware Composed: Independent Flip` — апаратний
  оверлей) → ні. Опис режимів — таблиця в `README-ConsoleApplication.md` того ж тегу.
- `rules::composed_present`: режим відомий у ≥ 50 % кадрів і композиція в ≥ 60 % з них;
  `Impact::Info`, `Category::Display`, впевненість 70 + 25 × частка, з порадою.
- Не перевірено на реальній грі у вікні — потрібен запис користувачем.

## 2c. Звіт для відгуку (у релізах)

- `crates/app/src/report.rs` — чисті функції з тестами: `redact` (заміна USERPROFILE /
  USERNAME / COMPUTERNAME без урахування регістру ASCII, лише цілими словами, імена < 3
  символів не чіпаються) і `summarize_cleanup_journal` (останні 20 сеансів, замість `lines`
  — кількість і до 20 рядків не `deleted/quarantined/restored`).
- `commands/feedback.rs`: `feedback_report` (пише `reports\optimax-report-<дата>-<unix>.json`),
  `feedback_reveal` (лише файли з теки звітів). `AppState.log_tail` (1000 рядків, наповнює
  потік `optimax-ui-log`) і `AppState.last_diagnosis` (пишеться перед `diag-done`).
- Нових залежностей немає (zip свідомо не робили — один JSON).
- Далі за планом користувача: WHEA/TDR із журналу подій, потім екран «Автозавантаження».

## 2d. Помилки заліза з журналу подій (у релізах)

- `audit/src/win/events.rs` — `EvtQuery` + `EvtRender` (системні властивості й EventData),
  XPath з `timediff(@SystemTime)`; ліміт 2000 подій. Тест `reads_real_system_log` читає
  справжній журнал (лише читання).
- `rules::check_hardware_events` + `whea_kind` (класифікація ID за маніфестом провайдера:
  описи «fatal / corrected», компонент; ID 29 — Warning, але fatal). Знахідки: `hw.tdr`
  (Moderate, ≥ 3 за період — High), `hw.whea_fatal` (High), `hw.whea_corrected` (Moderate),
  `hw.none` (Info). Нова категорія `Category::HardwareErrors`, нова перевірка
  `CheckId::HardwareEvents`; `SystemProbe::hardware_events` за замовчуванням `Ok(None)`.
- Не перевірено на ПК, де TDR/WHEA справді є (на ПК користувача їх немає за 90 днів).
- Далі за планом: екран «Автозавантаження».

## 2e. Екран «Автозавантаження» (у релізах)

- `tweaks/src/startup.rs` — чисті функції: `StartupSource` (5 джерел → ключ Run / тека і ключ
  StartupApproved), `is_enabled` (непарний перший байт = вимкнено; відсутнє = увімкнено),
  `approved_value` (12 байт як у Диспетчера задач), `exe_from_command`, `is_system_path`.
- `Desired::StartupApp` / `TweakId::StartupApp`; рушій перевіряє: запис у Run існує й має
  тип REG_SZ, exe не в `%SystemRoot%`, ім'я з теки — без шляху, прапорець — REG_BINARY.
  Відкат відновлює точне старе значення (або видаляє, якщо його не було).
- `app/src/commands/startup.rs`: `startup_list` (навантаження — `sysinfo` за шляхом exe;
  ярлики .lnk не розгортаються, тож для них «не працює» = не з'ясовано), `validate` для
  записів із тек (викликається в `tweak_plan`).
- Відомо: у «Резервних копіях» значення показуються як `REG_BINARY [..]` (без людського опису).
- Не перевірено на реальному Windows: що Explorer поважає наш запис при наступному вході й що
  Диспетчер задач показує зміну — чекліст у звіті.

## 2f. Оновлення з програми (у релізах)

- `tauri-plugin-updater` ~2.13.3 (2.14.1 свідомо не взято — 2 дні від виходу). Ініціалізація в
  `lib.rs`, конфіг — `plugins.updater` у `tauri.conf.json` (endpoint
  `releases/latest/download/latest.json`, `installMode: passive`), `createUpdaterArtifacts: true`.
- `pubkey` — з `%USERPROFILE%\.tauri\optimax.key.pub` (ключ згенеровано 2026-10-31 для 0.1.2;
  приватний ключ і пароль — лише в користувача). Копії 0.1.1 мають заглушку замість ключа й
  оновитися з програми не можуть — 0.1.2 ставиться вручну один раз.
- `commands/update.rs`: `update_check`, `update_install(version)` (повторно перевіряє версію й
  адресу, лише для встановленої копії), `update_open_page`. «Встановлено» = тека exe збігається
  з `InstallLocation` у ключі `Uninstall\OptimaX` (HKLM або HKCU); портативний exe Tauri теж
  позначає як nsis, тому за типом бандла визначати не можна.
- `requireSignedVersion: true` (захист від даунгрейду): CLI 2.12.4 пише `version:` у довірений
  коментар підпису (перевірено на `.sig` релізу 0.1.2). Workflow перевіряє, що прапорець увімкнено.
- Налаштування `update_check_on_start` (за замовчуванням false).

## 2g. Аудит безпеки й виправлення (0.1.2, закомічено п'ятьма комітами `fix(security): …`)

Модель загрози: код без прав адміністратора в сесії того самого користувача → процес OptimaX
з правами адміністратора (межа, яку проєкт сам задекларував: `requireAdministrator`, CLAUDE.md
11.3). Аудит — скілом `security-audit` у режимі поради (`~/.claude/skills/security-audit` —
посилання на `~/.agents/skills`).

- **Журнал змін:** `engine::is_known_target` — відкат і `recover_pending` виконують лише цілі
  й типи значень, які рушій сам пише для `tweak_id`. Новий твік = додати його ціль сюди
  (інакше його записи не відкочуватимуться; тест `genuine_targets_of_every_tweak_are_accepted`).
- **Файлові операції очищення/карантину:** `optimax_core::safefs` — відкриття з
  `FILE_FLAG_OPEN_REPARSE_POINT`, звірка `GetFinalPathNameByHandleW`, видалення через
  дескриптор, перейменування — `NtSetInformationFile` відносно дескриптора теки (Win32
  `SetFileInformationByHandle` з `RootDirectory` дає `ERROR_INVALID_PARAMETER`). Тести на
  справжньому NTFS із `mklink /J`. Мок `MemFs` розкриває посилання в проміжних теках, як ОС.
- **Тека даних:** `optimax_core::datadir` — `%ProgramData%\OptimaX\<SID>` (SY/BA повний,
  користувач — читання), спільна `%ProgramData%\OptimaX\bin` для PresentMon (лише SY/BA). Тека
  не з власником BA/SY або junction → відсувається в `*.untrusted-<час>` непрочитаною.
  Одноразове перенесення з `%LOCALAPPDATA%\OptimaX` — `app/src/migrate.rs` (типізовано, до
  32 МБ, без посилань; мітка `migrated-from-localappdata.json`). Карантин на іншому диску —
  `X:\OptimaX\<SID>\Quarantine`. `frametime::acl` прибрано.
- **Системні шляхи:** `optimax_core::sysdirs` (ядро + `SHGetKnownFolderPath`) замість змінних
  середовища для чорного списку, правил, Dism/powercfg/explorer, `nvml.dll`, тек
  автозавантаження. `LOCALAPPDATA`/`APPDATA` для правил — лише всередині профілю.
- **Не перевірено з правами адміністратора:** тест `datadir::…planted_dir_and_junction_are_moved_aside`
  (без прав пропускається); реальне очищення/карантин/відновлення; перенесення даних.
- **Лишилось:** звіт для відгуку містить командні рядки автозапуску —
  попереджати користувача переглянути перед відправкою.

## 3x. Модуль «Очищення» (історія, злито в `main`): гілка `feature/cleaning` (модуль «Очищення», CLAUDE.md розд. 11)

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
cargo test --workspace            # зараз 376 тестів
cd ui && npm run check && npm run check:i18n
```
