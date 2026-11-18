# Вердикт простими словами — план реалізації

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Зверху результату запису — розповідь людською мовою «Як іде гра / Чому були ривки / Що
зробити», складена бекендом із наявних метрик; інженерний вердикт і графіки — у згортках.

**Architecture:** Новий модуль `crates/diagnosis/src/story.rs` (чисті функції) будує `Story` усередині
`diagnose_frames`, коли ще доступні `Analysis` і структуровані причини ривків; класифікація причин
виноситься з `rules::stutter` у спільну функцію. `Diagnosis.story: Option<Story>` іде в UI через
ts-rs; `ResultView.svelte` показує нову `StoryCard.svelte`, а нинішній верх — у згортці «Детальніше».

**Tech Stack:** Rust (serde, ts-rs), Svelte 5 + TypeScript, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-18-plain-verdict-design.md`

## Global Constraints

- **Git робить лише користувач.** Жодних `git add/commit/push/checkout/reset`; «коміт»-кроки нижче —
  це зупинка з переліком змінених файлів. Дозволено лише `git status/diff/log`.
- Без `unwrap()/expect()/panic!` у продакшн-коді; `#![forbid(unsafe_code)]`; doc-коментарі на публічних типах і функціях.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (усе workspace).
- `diagnosis` — без системних викликів. Нових залежностей немає.
- Нових порогів немає: оцінки лише з наявних правил (`rules::stutter`, `Bottleneck`, `Impact`).
- Причина ривка — збіг у часі: «найімовірніше», «збіглося з», «N з M»; не «тому що».
- UI: лише токени `ui/src/styles/tokens.css`, вердикт — текстом і іконкою, українська, рядки в `crates/core/locales/uk.json`; `npm run check:i18n` проходить.
- Генерація TS-типів: `cargo test -p optimax-app` (оновлює `ui/src/lib/bindings/`).
- LF-закінчення рядків (`.gitattributes eol=lf`).

## Review Focus

1. Запис без частоти монітора (`display_hz = None`) — речення про FPS без «монітор — 0 Гц». Тест у Task 1.
2. Ривків менше 3 (`correlated_causes` нічого не рахує) при `Noticeable` — `why_unknown`, а не порожній блок без пояснення. Тест у Task 2.
3. Кілька знахідок з однаковим id (`background` ×2) — `action`/`others` указують на різні індекси, кожен — знахідка з `advice`. Тест у Task 1.
4. Старий JSON без поля `story` (фікстури, кеш) — десеріалізується, UI показує запасний вигляд. Тести в Task 1 (serde) і Task 4 (e2e).
5. Розповідь і знахідка `stutter.*` розходяться (різні причини/частки) — тест узгодженості в Task 2.

---

### Task 1: Тип `Story`, плавність, «Як іде гра», «Що зробити»

**Files:**
- Create: `crates/diagnosis/src/story.rs`
- Modify: `crates/diagnosis/src/lib.rs` (mod, поле `story`, виклик у `diagnose_frames` і в обох гілках «недостатньо даних»)
- Modify: літерали `Diagnosis { .. }` — `crates/app/src/commands/dashboard.rs:231`, `crates/app/src/commands/history.rs:383`, `crates/diagnosis/src/resolution.rs:497` (додати `story: None`)
- Modify: `crates/app/src/bindings.rs` (експорт `Story`, якщо типи експортуються списком), `crates/core/locales/uk.json`
- Test: `crates/diagnosis/src/tests.rs` (наявні хелпери `timeline`, `jittery`, `spiky`, `hitches_at_start`, `base_sample`, `find`)

**Interfaces:**
- Produces (усі з `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]` і `#[cfg_attr(feature = "ts", derive(ts_rs::TS))]`, enum-и `#[serde(rename_all = "snake_case")]`):
  - `pub struct Story { smoothness: Smoothness, outcome: StoryOutcome, how: Vec<Text>, why: Vec<StoryCause>, why_unknown: bool, action: Option<u32>, others: Vec<u32> }`
  - `pub enum Smoothness { Smooth, Isolated, Noticeable, Severe }`
  - `pub enum StoryOutcome { Fine, Fixable, Unexplained, Insufficient }`
  - `pub struct StoryCause { code: String, text: Text, likely: bool, hits: u32, of: u32 }`
  - `Diagnosis.story: Option<Story>` з `#[serde(default)]`
  - `pub(crate) fn compose(a: &Analysis<'_>, bottleneck: Bottleneck, primary_conf: u8, verdicts: &[Verdict]) -> Story` — у цій задачі `why` завжди порожній, `why_unknown = smoothness ≥ Noticeable` (Task 2 заповнить `why`)
  - `pub fn insufficient_story() -> Story` — `outcome: Insufficient`, `how: [diag.story.how.insufficient]`, решта порожня
- Правила (зі спеки, дослівно):
  - `smoothness` — з знахідки `stutter.*` у `verdicts`: немає → `Smooth`; `stutter.isolated` → `Isolated`; інший `stutter.*` з `Impact::Moderate` → `Noticeable`, з `Impact::High` → `Severe`.
  - кандидати дій — індекси `verdicts[1..]` з `advice.is_some()`, у наявному порядку. `Fixable`: є кандидат ≠ `stutter.unknown` з `impact ≥ Moderate` → `action` = перший такий, `others` = решта кандидатів. Інакше `Unexplained`, якщо `why_unknown` → `action` = індекс `stutter.unknown`, `others` = решта. Інакше `Fine` → `action = None`, `others` = усі кандидати. `Bottleneck::InsufficientData` → `insufficient_story()`.
  - `how` (ключі й аргументи):
    1. `FrameCap` → `diag.story.how.cap {avg}`; інакше `display_hz: Some(hz > 0)` → `diag.story.how.fps_hz {avg, hz}`; інакше `diag.story.how.fps {avg}`. `avg` = `format!("{:.0}", stats.avg_fps)`.
    2. `diag.story.how.smooth` / `.isolated {spikes, first}` / `.noticeable {low, avg, spikes}` / `.severe {low, avg, spikes, max}` (`first` — секунда першого ривка з 1 знаком, як у `isolated_hitches`; `max` = `max_frametime_ms` без знаків).
    3. `Gpu` → `diag.story.how.gpu` (або `.gpu_borderline`, якщо `primary_conf <= 60`), `Cpu` → `.cpu`, `FrameCap` → `.cap_reason`, `Unclear` → `.unclear`.
  - Тексти `uk.json` — за спекою («Як іде гра»); `isolated` — та сама думка, що в `diag.stutter.isolated.detail`, не сильніша. Ключі `diag.story.outcome.{fine,fixable,unexplained,insufficient}` (заголовок картки), `diag.story.smooth.{smooth,isolated,noticeable,severe}` (бейдж).

- [ ] **Step 1: Write the failing tests** у `tests.rs`:

```rust
fn story(d: &Diagnosis) -> &Story { d.story.as_ref().expect("розповідь") }
fn keys(s: &Story) -> Vec<&str> { s.how.iter().map(|t| t.key.as_str()).collect() }

#[test] fn story_smooth_gpu_is_fine() {
    // gpu_bound_scenario-подібний таймлайн без піків, display_hz 144
    // assert smoothness Smooth, outcome Fine, action None,
    // keys == ["diag.story.how.fps_hz", "diag.story.how.smooth", "diag.story.how.gpu"],
    // how[0].args["hz"] == "144"; others містить індекс upscale.helps; кожен індекс others має advice
}
#[test] fn story_without_display_hz_omits_monitor() { /* Context::default() → how[0].key == "diag.story.how.fps", немає arg "hz" */ }
#[test] fn story_isolated_hitches() { /* hitches_at_start → Isolated, how[1] key .isolated, args spikes "3" */ }
#[test] fn story_stutter_without_cause_is_unexplained() {
    /* spiky() без ознак → Noticeable|Severe (за impact знахідки stutter.unknown), why_unknown,
       outcome Unexplained, verdicts[action].finding.id == "stutter.unknown" */
}
#[test] fn story_frame_cap() { /* vsync_cap_is_not_cpu_bound-таймлайн → how[0].key "diag.story.how.cap" */ }
#[test] fn story_fixable_points_at_advice() {
    /* thermal_throttling (throttled helper) або background×2 → outcome Fixable,
       verdicts[action] має advice і impact ≥ Moderate і id != "stutter.unknown";
       action ∉ others; others без дублів */
}
#[test] fn story_insufficient() { /* insufficient_data-сценарій → outcome Insufficient, why порожній, action None */ }
#[test] fn story_keys_exist_in_catalog() {
    /* для всіх сценаріїв вище: Catalog::ukrainian()?.contains(key) для кожного how.key */
}
#[test] fn diagnosis_without_story_deserializes() {
    /* serde_json::to_value(d) → видалити "story" → from_value::<Diagnosis> Ok, story == None */
}
```

- [ ] **Step 2: Run** `cargo test -p optimax-diagnosis story` — Expected: FAIL (компіляція: немає `Story`, `story`).
- [ ] **Step 3: Implement** `story.rs`, поле в `Diagnosis`, виклик `story::compose(&a, bottleneck, primary_conf, &verdicts)` після збирання `verdicts` у `diagnose_frames`; `Some(insufficient_story())` в обох гілках недостатніх даних; `story: None` у трьох літералах поза `lib.rs`; тексти в `uk.json`.
- [ ] **Step 4: Run** `cargo test -p optimax-diagnosis` — Expected: PASS (нові й наявні). Потім `cargo test --workspace` і `cargo clippy --all-targets -- -D warnings` — PASS.
- [ ] **Step 5: Stop for user commit** — перелік змінених файлів.

### Task 2: «Чому були ривки» — спільна класифікація причин

**Files:**
- Modify: `crates/diagnosis/src/rules.rs` (`Cause`, `correlated_causes`, `stutter`, `spike_driver`)
- Modify: `crates/diagnosis/src/story.rs`, `crates/core/locales/uk.json`
- Test: `crates/diagnosis/src/tests.rs`

**Interfaces:**
- Consumes: `compose` з Task 1.
- Produces:
  - `pub(crate) struct StutterCauses { pub main: Vec<Cause>, pub partial: Vec<Cause>, pub spikes: usize, pub driver: Option<String> }` і `pub(crate) fn stutter_causes(a: &Analysis<'_>) -> StutterCauses` у `rules.rs`; поля `Cause` — `pub(crate)`. `spikes = a.spike_times.len()`; `driver` — назва драйвера, яку `stutter` уже називає для DPC (та сама умова). `stutter()` використовує саме цю функцію (без другого обчислення); «як завжди» `stutter` і далі рахує сам або через ту саму функцію — на розсуд, але межі ті самі.
  - `compose` заповнює `why`: спершу `main` (`likely: true`, ключ `diag.story.why.likely.<code>`), далі `partial` (`likely: false`, `diag.story.why.partial.<code>`), не більше 3; `hits = (at_spikes * spikes as f64).round() as u32`, `of = spikes as u32`; аргументи тексту `hits`, `of`; DPC з `driver: Some(name)` → ключ `diag.story.why.likely.dpc_driver` / `.partial.dpc_driver` з аргументом `name`. `code` = `CauseKind::code()`.
  - `why_unknown = smoothness ∈ {Noticeable, Severe} && why.is_empty()`; `Smooth`/`Isolated` → `why` порожній.
  - Тексти: 9 причин × 2 + 2 для драйвера + `diag.story.why.unknown` («Причину не знайдено: ривки ні з чим виміряним не збіглися»).

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn story_names_disk_as_likely_cause() {
    // як stutter_attributed_to_disk
    // let s = story(&d); assert_eq!(s.why[0].code, "disk"); assert!(s.why[0].likely);
    // assert_eq!(s.why[0].of as usize, spike_times(&ft).len()); assert!(s.why[0].hits >= s.why[0].of / 2);
    // assert!(!s.why_unknown); assert!(s.why.iter().all(|c| c.code != "background"));
}
#[test] fn story_partial_cause_is_not_likely() {
    // диск лише біля ~третини піків (ознака вдвічі частіша за звичайне) → why[0].likely == false, stutter.partial у verdicts
}
#[test] fn story_and_stutter_agree() {
    // для сценаріїв disk / partial / shader (shader_cache_writes_near_spikes...) / defender:
    // множина why.code (likely) == множина CauseKind з доказів stutter.found (label diag.cause.*),
    // множина why.code (!likely) == з доказів diag.cause_name.* з value diag.ev.partial_share
}
#[test] fn story_few_spikes_noticeable_is_unknown() {
    // 1% low < 50% avg, але spike_times < 3 (correlated_causes порожній) → якщо smoothness Noticeable|Severe, то why_unknown
}
#[test] fn story_why_keys_exist_in_catalog() { /* кожен why[].text.key і diag.story.why.unknown є в Catalog */ }
```

- [ ] **Step 2: Run** `cargo test -p optimax-diagnosis story` — Expected: FAIL (`why` порожній).
- [ ] **Step 3: Implement** `stutter_causes`, переведення `stutter` на неї, заповнення `why` у `compose`, тексти.
- [ ] **Step 4: Run** `cargo test -p optimax-diagnosis` — Expected: PASS, зокрема всі наявні `stutter_*`, `defender_*`, `memory_*`, `shader_*`, `vram_*` (поведінка `stutter` не змінилась). Потім `cargo test --workspace`, clippy — PASS.
- [ ] **Step 5: Stop for user commit.**

### Task 3: Очікування на записах RE9 і типи для UI

**Files:**
- Modify: `crates/diagnosis/tests/fixture/mod.rs` (`Expect` + `check`), `crates/diagnosis/tests/recordings/re9-*.toml`, `crates/diagnosis/tests/recordings/README.md` (описати нові поля)
- Generated: `ui/src/lib/bindings/{Story,Smoothness,StoryOutcome,StoryCause,Diagnosis}.ts`

**Interfaces:**
- Produces: `Expect.smoothness: Option<String>`, `Expect.outcome: Option<String>` (snake_case назви варіантів); `check` додає «плавність: очікувалось X, вийшло Y» / «підсумок: …», і «немає розповіді», якщо `story` = `None`.

- [ ] **Step 1:** додати поля в `Expect` і перевірку в `check`; у `harness_roundtrip_and_check` — випадок, коли очікувана плавність не збігається → `check` повертає помилку.
- [ ] **Step 2: Run** `cargo test -p optimax-diagnosis --test recordings` — Expected: FAIL лише в новому випадку roundtrip-тесту, поки `check` не дописаний; після — PASS.
- [ ] **Step 3:** тимчасово надрукувати `story` трьох записів (ігнорований тест або `--nocapture`), звірити з `[expect]` і описом кожного запису (ліміт 60 → `cap`, DLSS — GPU), вписати `smoothness`/`outcome` у три `.toml`. Якщо значення суперечить опису запису — **зупинитись і повідомити користувача**, не підганяти. Тимчасовий друк прибрати.
- [ ] **Step 4: Run** `cargo test -p optimax-diagnosis --test recordings` — PASS; `cargo test -p optimax-app` — PASS, згенеровано `Story.ts` та ін., `Diagnosis.ts` має `story: Story | null`.
- [ ] **Step 5: Stop for user commit.**

### Task 4: `StoryCard` і новий верх `ResultView`

**Files:**
- Create: `ui/src/components/StoryCard.svelte`
- Modify: `ui/src/components/ResultView.svelte`, `ui/src/lib/tour.svelte.ts` (лише якщо цілі змінюються), `ui/src/screens/Diagnosis.svelte:92` (прокрутка до `diag-stats`), `crates/core/locales/uk.json`
- Modify tests: `ui/tests/e2e/fixtures.ts` (`gpuResult.diagnosis.story`), `ui/tests/e2e/diagnosis.spec.ts`, `ui/tests/e2e/diagnosis-premium.spec.ts`
- Create test: `ui/tests/e2e/story.spec.ts`

**Interfaces:**
- Consumes: `Story` з bindings; `Diagnosis.verdicts[i].finding` (`advice`, `next`) для дій.
- Produces: `StoryCard` props `{ story: Story; verdicts: Verdict[]; meta?: Snippet; action?: Snippet<[Finding]> }` — `action` рендерить наявний `stepAction(f)` з `ResultView` (кнопки compare / upscale без дублювання).
- UI-ключі: `ui.story.how` «Як іде гра», `ui.story.why` «Чому були ривки», `ui.story.todo` «Що зробити», `ui.story.others` «Інші варіанти», `ui.story.nothing` «Нічого робити не треба», `ui.diag.details` «Детальніше» (+ підсумок згортки, напр. «Показники, вердикт і докази»).
- Розмітка (спека, «Інтерфейс»): `StoryCard` (бейдж плавності `diag.story.smooth.*` з іконкою, заголовок `diag.story.outcome.*`, три блоки, «Інші варіанти» — `Disclosure` згорнуто, рядок `meta` — зарахований час і `end_reason` з нинішнього підвалу `KpiStrip`) → `ComparisonSummary` як зараз → `Fold` «Детальніше» (`KpiStrip`, `HeroCard` з кільцем і доказами, список `FindingCard`) → `Fold` «Для досвідчених» без змін → `after`. `story == null` → нинішній вигляд без змін. `data-tour="diag-summary"` → на `StoryCard`; `diag-stats` → на блок «Як іде гра» (крок туру `optional` лишається).
- Фікстура `gpuResult.diagnosis.story`: `{ smoothness: "smooth", outcome: "fine", how: [{key:"diag.story.how.fps_hz",args:{avg:"125",hz:"144"}}, {key:"diag.story.how.smooth",args:{}}, {key:"diag.story.how.gpu",args:{}}], why: [], why_unknown: false, action: null, others: [1, 2] }`.

- [ ] **Step 1: Write failing e2e** `story.spec.ts` (за зразком `game-library.spec.ts`: `mockBackend`, `emit(page, "diag-done", …)` або `history_open`):
  - «розповідь видно одразу»: видно `tk("ui.story.how")`, текст `diag.story.how.fps_hz` з 125/144, `tk("diag.story.outcome.fine")`, `tk("ui.story.nothing")`; вміст кільця/`ui.diag.check` прихований.
  - «Детальніше розгортає вердикт»: клік `tk("ui.diag.details")` → видно `KpiStrip` (`ui.diag.kpi.label`) і заголовок `summary.gpu`.
  - «ривки з причиною»: копія `gpuResult` з `smoothness: "noticeable"`, `outcome: "fixable"`, `why: [{code:"shader", likely:true, hits:7, of:10, text:{key:"diag.story.why.likely.shader",args:{hits:"7",of:"10"}}}]`, `action: 1` → видно текст причини і пораду `diag.gpu_bound.advice` з кнопкою `ui.diag.next.compare`, клік викликає `onCompare` (перехід у «Порівняння», як у наявних тестах).
  - «Інші варіанти»: при `others: [2]` після розгортання — порада апскейлу й кнопка `ui.up.open`.
  - «без розповіді — старий вигляд»: `story: null` → `KpiStrip` видно без розгортання.
- [ ] **Step 2: Run** `cd ui; npx playwright test story` — Expected: FAIL (немає `StoryCard`).
- [ ] **Step 3: Implement** `StoryCard.svelte`, зміни `ResultView`, тексти, фікстуру; у `diagnosis.spec.ts` / `diagnosis-premium.spec.ts` там, де тест перевіряє верх результату (кільце, `ui.diag.check`, KPI), спершу розгорнути «Детальніше» — суть перевірок не змінювати.
- [ ] **Step 4: Run** `cd ui; npm run check; npm run check:i18n; npx playwright test` — Expected: PASS усе.
- [ ] **Step 5: Stop for user commit.**

### Task 5: Документація й підсумкова перевірка

**Files:**
- Modify: `CHANGELOG.md` ([Unreleased]), `docs/HANDOFF.md` (новий розділ стану: етап 2 зроблено, далі етап 3), за потреби `docs/DESIGN.md` (розд. 6 — новий порядок блоків результату)

- [ ] **Step 1:** оновити документи.
- [ ] **Step 2: Run** `cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test --workspace; cargo check --workspace --target x86_64-pc-windows-msvc` і `cd ui; npm run check; npm run check:i18n; npx playwright test` — Expected: усе PASS.
- [ ] **Step 3:** звіт користувачу: що зроблено, що покрито тестами, що **не** перевірено на реальному Windows, чекліст ручної перевірки (записати гру з ривками й без; порівняти розповідь із шкалою в «Для досвідчених»; відкрити старий запис з «Історії»; тур знайомства).
- [ ] **Step 4: Stop for user commit.**
