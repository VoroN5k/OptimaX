# «Спробувати» — необов'язкова дія в «Перевірити мою гру» — план

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Після «Усе гаразд» з порадами сценарій не обривається: поради видно одразу, «Спробувати» будь-яку з них веде до «Було / стало» за метрикою саме цієї поради; без порад — завершальна картка замість сірої плашки.

**Architecture:** Бекенд: `check_plan` приймає id обраної знахідки, `CheckPlan.advice` несе текст поради до «Було / стало»; `diag_start` отримує `check_finding`. UI: `StoryCard` показує поради `Fine` списком і кнопку «Спробувати» (лише в сценарії), вибір живе в `ui.check.choice`.

**Tech Stack:** Rust (optimax-diagnosis, optimax-history, optimax-app, ts-rs), Svelte 5 + TS, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-19-check-my-game-design.md`, розділ «Доповнення 2026-11-20».

## Global Constraints

- Git — лише користувач (кроки «Commit» = зелений набір тестів).
- Нових залежностей, порад і порогів немає; висновок — `history::check` без змін.
- Логіка вибору плану — у бекенді; тексти — `uk.json`; `npm run check:i18n` чистий.
- Нові поля serde, що потрапляють в `history/*.json`, — `#[serde(default)]`.
- Без `unwrap/expect/panic` у продакшн-коді; fmt/clippy `-D warnings`; лише токени в стилях.

## Review Focus

1. Запис «було» з «Історії» має інший порядок знахідок → план за id тієї самої знахідки, не за індексом (тест у Task 1).
2. Ланцюжок: друге «Я зробив» з іншим обраним варіантом не бере план першого з кешу (тест у Task 2).
3. `Fine` без порад у сценарії → завершальна картка, без «Спробувати» і без «Я зробив» (тест у Task 3).
4. Вибір варіанта, потім повторний запис «було» («Записати ще раз» чи звичайний запис) → вибір скидається (тест у Task 3).
5. Поза сценарієм «Спробувати» немає ніде (зокрема в «Історії») (тест у Task 3).

---

### Task 1: план для обраної знахідки (`diagnosis`, `history`)

**Files:** `crates/diagnosis/src/check.rs`, `crates/history/src/compare.rs` (`CheckResult.advice`), тести в них.

**Interfaces:**
- `pub fn check_plan(d: &Diagnosis, finding: Option<&str>) -> Option<CheckPlan>`:
  - `None` → як зараз (головна дія при `Fixable`/`Unexplained`);
  - `Some(id)` → знахідка з цим id серед `story.action` і `story.others`, з порадою; `outcome` будь-який, крім `Insufficient`; немає такої → `None`.
- `CheckPlan.advice: Option<Text>` (`#[serde(default)]`) — порада обраної знахідки.
- `CheckResult.advice: Option<Text>` — з `plan.advice`.

- [ ] **Step 1: тести** `plan_for_chosen_other_in_fine` (Fine, `others = [1]`, `Some("finding")` → план з `advice`), `plan_by_id_not_index` (знахідки в іншому порядку — той самий план), `plan_unknown_or_unlisted_id_is_none`, `plan_chosen_in_insufficient_is_none`, `plan_main_has_advice`; `check_carries_advice` у `history`.
- [ ] **Step 2: Run** `cargo test -p optimax-diagnosis check; cargo test -p optimax-history` — Expected: FAIL (компіляція).
- [ ] **Step 3:** реалізувати; наявні виклики `check_plan(&d)` → `check_plan(&d, None)`.
- [ ] **Step 4: Run** ті самі команди — Expected: PASS.

### Task 2: `diag_start(.., check_finding)` (`app`)

**Files:** `crates/app/src/commands/{diagnosis.rs,history.rs}`, `crates/app/src/state.rs`; bindings через `cargo test -p optimax-app`.

**Interfaces:**
- `diag_start(exe, seconds, scene, check_of: Option<String>, check_finding: Option<String>)`.
- `plan_for_check(baseline, exe, finding: Option<&str>) -> CmdResult<CheckPlan>`.
- Кеш `AppState.last_check: Mutex<Option<(String /*game*/, Option<String> /*finding*/, CheckLink)>>`; `cached_plan(check_of, finding, cached)` — лише коли збігаються і `baseline`, і `finding`.

- [ ] **Step 1: тести** `plan_for_check_chosen_finding` (Fine-результат + id іншого варіанта → `Ok` з `advice`), `cached_plan_needs_same_finding`.
- [ ] **Step 2: Run** `cargo test -p optimax-app` — Expected: FAIL.
- [ ] **Step 3:** реалізувати.
- [ ] **Step 4: Run** `cargo test -p optimax-app; cargo clippy --all-targets -- -D warnings` — Expected: PASS, `CheckPlan.ts`/`CheckResult.ts` з `advice`.

### Task 3: інтерфейс

**Files:** `ui/src/components/{StoryCard,ResultView,CheckCard}.svelte`, `ui/src/screens/Diagnosis.svelte`, `ui/src/lib/{api.ts,state.svelte.ts}`, `ui/src/components/CheckStart.svelte`, `crates/core/locales/uk.json`, `ui/tests/e2e/{check,story}.spec.ts`.

**Interfaces / поведінка:**
- `api.diagStart(exe, seconds, scene, checkOf = null, checkFinding = null)` — `checkFinding` в аргументах лише коли не `null`.
- `CheckSession.choice: string | null` — id обраної поради; скидається в `start()` для нового «було».
- `StoryCard` нові пропси: `onTry?: (f: Finding) => void`, `tried?: string | null`. Без головної дії й з порадами в `others` — блок «Що зробити» з `ui.story.try_title` і нумерованим списком порад (без згортки); з головною дією — як зараз. З `onTry` — біля кожної поради (головної й інших) кнопка «Спробувати»; `tried` — позначено «Обрано» (текстом).
- `ResultView` прокидає `onTry`/`tried` у `StoryCard`; Diagnosis передає їх лише в сценарії.
- Diagnosis: `canRecheck` = є «було» і (`checkable` або `choice`); блок «Зроби це й перевір» показує обрану пораду (`t(advice)`) або головну; «Я зробив» шле `checkFinding = choice`. Крок 4 — є «було» й (дія чи вибір); `final` — `Fine` без порад.
- `Fine` без порад у сценарії — завершальна картка `ui.check.done_*` з кнопками «Завершити» (`ui.check = null`) і «Перевірити іншу гру» (новий сценарій, налаштування запису розгорнуто).
- `CheckCard`: є `res.advice` → рядок «Що змінено:» + текст поради.
- Тексти (точні): `ui.story.try_title` «Хочеш ще краще? Спробуй (необов'язково):»; `ui.story.try` «Спробувати»; `ui.story.tried` «Обрано»; `ui.check.changed` «Що змінено:»; `ui.check.done_title` «Перевірку завершено»; `ui.check.done_text` «Несправностей немає, і порад для цієї гри теж немає. Перевір ще раз після оновлення драйвера чи гри.»; `ui.check.another` «Перевірити іншу гру». `ui.check.nothing` — прибрати.

- [ ] **Step 1: e2e**: `story.spec` — `Fine з порадами — список «Спробуй», без «Нічого робити не треба» і без згортки`; `check.spec` — `«Спробувати» варіант після «Усе гаразд» → «Я зробив» шле checkFinding`, `«Спробувати» інший варіант при головній дії`, `Fine без порад — завершальна картка, «Перевірити іншу гру»`, `новий «було» скидає вибір`, `поза сценарієм немає «Спробувати»`, `«Було / стало» показує «Що змінено»`; оновити тест `«Усе гаразд» завершує сценарій` під новий вигляд.
- [ ] **Step 2: Run** `cd ui; npx playwright test check story` — Expected: FAIL.
- [ ] **Step 3:** реалізувати.
- [ ] **Step 4: Run** `npm run check; npm run check:i18n; npx playwright test` — Expected: усе PASS.

### Task 4: документи й перевірка

- [ ] `CHANGELOG.md` (доповнити пункт «Перевірити мою гру»), `docs/HANDOFF.md` (розділ 2y), `docs/DESIGN.md` (нотатка розд. 6).
- [ ] **Run** `cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test --workspace; cargo check --workspace --target x86_64-pc-windows-msvc`; `cd ui; npm run check; npm run check:i18n; npx playwright test` — Expected: усе PASS.
