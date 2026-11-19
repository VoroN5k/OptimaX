# «Перевірити мою гру» з «було / стало» — план

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Сценарій «Огляд» → «Діагностика»: гра → запис 120 с → розповідь → одна дія → повторний запис → «було / стало» з висновком за метрикою дії і пропозицією A/B, коли один запис нічого не довів.

**Architecture:** Бекенд вирішує все: `diagnosis::check_plan` (метрика й варіанти A/B з головної дії розповіді «було»), `history::check` (висновок за метрикою з наявним порогом), зв'язок `HistoryEntry.check` пишеться під час запису з `check_of`. Інтерфейс: картка на «Огляді», стан сценарію в `ui.check` (сесія), смужка кроків і блок «Зроби це й перевір» у «Діагностиці», картка «Було / стало» в `ResultView` замість «Проти попереднього запису».

**Tech Stack:** Rust (optimax-diagnosis, optimax-history, optimax-app, ts-rs), Svelte 5 + TS, Playwright з мок-бекендом.

**Spec:** `docs/superpowers/specs/2026-11-19-check-my-game-design.md`

## Global Constraints

- Git робить лише користувач: жодних `git add/commit/...`; кроки «Commit» = зупинитися на зеленому наборі тестів (коміт зробить користувач після етапу).
- Нових залежностей немає. Нових порогів немає: `threshold_pct` — наявний з `history::compare` (похибка обох записів, не менше `MIN_SIGNIFICANT_PCT` = 8 %).
- `history::compare`, `Trend`, `regressions`, `previous_of` — без змін поведінки.
- Логіка висновку лише в бекенді (CLAUDE.md 10.8); тексти — `uk.json` (`ui.check.*`), `npm run check:i18n` чистий.
- Без `unwrap/expect/panic` у продакшн-коді; doc-коментарі на публічних типах; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- UI — лише токени `ui/src/styles/tokens.css`; стан кроку виражений текстом/номером, не лише кольором.
- Нові поля serde — `#[serde(default)]` (старі `history/*.json` читаються).

## Review Focus

1. Повторний запис іншої гри з `check_of` (людина змінила гру в налаштуваннях) → команда відмовляє, не пише хибне «було / стало». Тест у Task 3.
2. `history-saved` повторного запису не перезаписує «було» сценарію (ланцюжок — завжди з першим). Тест у Task 4.
3. Запис-перевірка, чий «було» видалено з «Історії» → `check_result` = `None`, картка не з'являється, без помилки. Тест у Task 2.
4. «Було» з `InsufficientData` (не збережено в історію) → кнопки «Я зробив» немає, є «Записати ще раз». Тест у Task 4.
5. 1% low відсутній в одному із записів при фокусі `Lows` → `Unknown` + A/B-пропозиція (якщо є варіанти), а не «краще». Тест у Task 2.

---

### Task 1: `check_plan` у `diagnosis`

**Files:**
- Create: `crates/diagnosis/src/check.rs`
- Modify: `crates/diagnosis/src/lib.rs` (`pub mod check; pub use check::{check_plan, CheckFocus, CheckPlan};`)
- Test: у `crates/diagnosis/src/check.rs` (`#[cfg(test)] mod tests`) — допоміжні конструктори діагнозу з `crates/diagnosis/src/tests.rs` за потреби зробити `pub(crate)`.

**Interfaces:**
- Produces:
  - `pub enum CheckFocus { Fps, Lows }` — serde snake_case, Copy, Eq, ts-derive.
  - `pub struct CheckPlan { pub focus: CheckFocus, pub ab: Option<[Text; 2]> }` — Clone, PartialEq, Serialize, Deserialize, ts-derive.
  - `pub fn check_plan(d: &Diagnosis) -> Option<CheckPlan>`.

Правило: `d.story` є, `outcome` — `Fixable` або `Unexplained`, `action = Some(i)`, `verdicts[i]` існує → `focus = Lows`, якщо `finding.category` ∈ {`Stutter`, `Background`, `Vram`}, інакше `Fps`; `ab = Some(variants)`, якщо `finding.next = Some(NextStep::Compare { variants })`. Інакше `None`.

- [ ] **Step 1: тести** `plan_fps_for_gpu_advice` (Fixable, дія `gpu_bound` з `next: Compare` → `Fps`, `ab` = ці варіанти), `plan_lows_for_stutter_background_vram` (кожна з трьох категорій → `Lows`), `plan_unexplained_is_lows` (`stutter.unknown`), `plan_none_when_fine_or_insufficient`, `plan_none_without_story`, `plan_none_when_action_index_out_of_range`, `plan_without_next_has_no_ab`.
- [ ] **Step 2: Run** `cargo test -p optimax-diagnosis check` — Expected: FAIL (немає модуля).
- [ ] **Step 3:** реалізувати.
- [ ] **Step 4: Run** `cargo test -p optimax-diagnosis` — Expected: PASS.

### Task 2: зв'язок і висновок у `history`

**Files:**
- Modify: `crates/history/src/model.rs` (`CheckLink`, поле `HistoryEntry.check`), `crates/history/src/compare.rs` (`CheckResult`, `check`, `latest_check`), `crates/history/src/lib.rs` (реекспорт)
- Modify: усі літерали `HistoryEntry { .. }` (тести `compare.rs`, `store.rs`, `crates/app/src/commands/history.rs::entry_from`) — `check: None`.

**Interfaces:**
- Consumes: `CheckFocus`, `CheckPlan` (Task 1).
- Produces:
  - `pub struct CheckLink { pub baseline: String, pub plan: CheckPlan }` (Serialize, Deserialize, ts).
  - `HistoryEntry.check: Option<CheckLink>` з `#[serde(default)]`.
  - `pub struct CheckResult { pub comparison: RecordComparison, pub focus: CheckFocus, pub trend: Trend, pub change_pct: Option<f64>, pub ab: Option<[Text; 2]> }` (Serialize, ts).
  - `pub fn check(baseline: &HistoryEntry, after: &HistoryEntry, plan: &CheckPlan) -> CheckResult` — `comparison = compare(baseline, after)`; `change_pct` = `avg_fps_change_pct` (Fps) чи `low_1pct_change_pct` (Lows); `trend` за `change_pct` і `comparison.threshold_pct` тими самими правилами, що в `compare` (`None` → `Unknown`); `ab` = `plan.ab` лише при `Same | Unknown`.
  - `pub fn check_of<'a>(entries: &'a [HistoryEntry], id: &str) -> Option<CheckResult>` — запис `id` з `check`, його `baseline` є в `entries` → `check(..)`; інакше `None`.
  - `pub fn latest_check(entries: &[HistoryEntry]) -> Option<CheckResult>` — найновіший за `at_unix_ms` запис, для якого `check_of` дає `Some`.

- [ ] **Step 1: тести** у `compare.rs`: `check_fps_better_at_threshold` (avg +8 % при поріг 8 → `Better`), `check_lows_uses_low_1pct` (avg +0 %, 1% low +30 % → `Better` для `Lows`, а `comparison.trend` = `Same`), `check_lows_missing_is_unknown_with_ab` (1% low `None` → `Unknown`, `ab` = варіанти), `check_worse_has_no_ab`, `check_of_missing_baseline_is_none`, `check_of_plain_entry_is_none`, `latest_check_picks_newest`; у `model.rs`/`store.rs`: `entry_without_check_deserializes` (JSON без поля `check`).
- [ ] **Step 2: Run** `cargo test -p optimax-history` — Expected: FAIL (компіляція).
- [ ] **Step 3:** реалізувати; винести обчислення тренду з `compare` у приватну `trend_of(change: Option<f64>, threshold: f64) -> Trend`, щоб правило було одне.
- [ ] **Step 4: Run** `cargo test -p optimax-history; cargo test -p optimax-app --no-run` — Expected: PASS / компілюється.

### Task 3: запис-перевірка й команди в `app`

**Files:**
- Modify: `crates/app/src/commands/diagnosis.rs` (`diag_start` + `check_of`), `crates/app/src/commands/history.rs` (`remember`/`entry_from` + `check`, `open_result`, `plan_for_check`, команди `check_result`, `check_last`), `crates/app/src/lib.rs` (реєстрація команд)
- Generated: `ui/src/lib/bindings/{CheckFocus,CheckPlan,CheckLink,CheckResult}.ts`, `HistoryEntry.ts` (через `cargo test -p optimax-app`)

**Interfaces:**
- Consumes: Task 1–2.
- Produces (Tauri):
  - `diag_start(exe, seconds, scene, check_of: Option<String>)` — з `check_of` на початку фонового потоку (до запису): `open_result(&id)` (тіло нинішнього `history_open`, винесене в sync-функцію), `plan_for_check(&baseline, &exe)`; помилка → подія `diag-error` з `CmdError::Failed`, запис не починається. Після запису `remember(.., scene, check)` з `Some(CheckLink { baseline: id, plan })`.
  - `pub(crate) fn plan_for_check(baseline: &DiagnosisResult, exe: &str) -> CmdResult<CheckPlan>`: гра не та сама (без регістру) → `Failed("запис «було» — інша гра")`; `check_plan` = `None` → `Failed("у записі «було» немає дії для перевірки")`. Немає сирого запису → наявний `NotFound("повний запис уже не зберігається")` з `open_result`.
  - `check_result(id: String) -> Option<CheckResult>` (`history::check_of`), `check_last() -> Option<CheckResult>` (`history::latest_check`) — async + `blocking`, як сусідні команди.

- [ ] **Step 1: тести** у `history.rs`: `plan_for_check_rejects_other_game`, `plan_for_check_rejects_fine`, `plan_for_check_ok` (повертає план Task 1), `entry_from_keeps_check_link`.
- [ ] **Step 2: Run** `cargo test -p optimax-app check` — Expected: FAIL.
- [ ] **Step 3:** реалізувати; `history_open` викликає `open_result`.
- [ ] **Step 4: Run** `cargo test -p optimax-app` (генерує bindings) і `cargo clippy --all-targets -- -D warnings` — Expected: PASS, нові `.ts` у `ui/src/lib/bindings`.

### Task 4: інтерфейс сценарію

**Files:**
- Create: `ui/src/components/CheckStart.svelte` (картка «Огляду»), `ui/src/components/CheckSteps.svelte` (смужка кроків), `ui/src/components/CheckCard.svelte` («Було / стало»), `ui/tests/e2e/check.spec.ts`
- Modify: `ui/src/lib/api.ts` (`diagStart(exe, seconds, scene, checkOf: string | null = null)` → `check_of`; `checkResult(id)`, `checkLast()`), `ui/src/lib/state.svelte.ts` (`check: null as CheckSession | null`, тип `{ baseline: string | null; exe: string; seconds: number; scene: string }`), `ui/src/screens/Analysis.svelte` (картка над «Станом ПК»), `ui/src/screens/Diagnosis.svelte`, `ui/src/components/ResultView.svelte`, `crates/core/locales/uk.json`, `ui/tests/e2e/fixtures.ts` (`check_result: null`, `check_last: null` у `defaults`)

**Interfaces:**
- Consumes: команди Task 3, bindings `CheckResult`, `CheckFocus`, `Trend`.
- Поведінка:
  - `CheckStart`: заголовок «Перевірити мою гру», текст, кнопка «Почати» → `ui.check = { baseline: null, exe: "", seconds: 120, scene: "" }`, `ui.screen = "diagnosis"`; під кнопкою — `checkLast()`: «Остання перевірка: {game} · {from} → {to} {метрика} · {висновок}», якщо є.
  - `Diagnosis.svelte` при `ui.check`: вкладка `lag`, `seconds = 120`, `setupOpen = true`; над вмістом `CheckSteps` (`step`: 1 — немає гри; 2 — запис/визначення; 3 — результат без «було»; 4 — є «було», результат — «було»; 5 — результат — запис-перевірка) і кнопка «Завершити перевірку» (`ui.check = null`). `history-saved` записує id у `ui.check.baseline` **лише якщо він `null`**, разом з `exe`, `seconds`, `ui.scene` першого запису.
  - Блок «Зроби це й перевір» (у `after` снипеті `ResultView`, лише в сценарії, `story.outcome` ∈ {fixable, unexplained}, `ui.check.baseline` є): порада головної дії, «Грай ту саму сцену, що й зараз», кнопка «Я зробив — записати ще раз» → `api.diagStart(check.exe, check.seconds, check.scene || null, check.baseline)`. `fine` → «Усе гаразд: перевіряти нічого» і кроки 4–5 неактивні. `insufficient` або немає `baseline` після результату → кнопка «Записати ще раз» (звичайний `begin`).
  - `ResultView`: для `historyId` спершу `checkResult(id)`; `Some` → `CheckCard` замість «Проти попереднього запису» (той самий `ComparisonSummary` + висновок), інакше як зараз.
  - `CheckCard`: висновок `ui.check.trend.{better|worse|same|unknown}.{fps|lows}` з `{from}`, `{to}`, `{pct}`, `{t}`; `better` → «Залиш так», `worse` → «Поверни як було»; `ab` → кнопка «Перевірити точно (A/B)» → `onCompare(game, ab.map(t))`.
- Тексти (`uk.json`, точні): `ui.check.title` «Перевірити мою гру»; `ui.check.text` «Вибери гру, пограй 2 хвилини — програма скаже, що заважає і чи допомогла зміна.»; `ui.check.start` «Почати»; `ui.check.last` «Остання перевірка: {game} · {from} → {to} · {verdict}»; `ui.check.steps.{game|record|verdict|action|result}` «Гра», «Запис», «Вердикт», «Дія», «Було / стало»; `ui.check.finish` «Завершити перевірку»; `ui.check.do_title` «Зроби це й перевір»; `ui.check.same_scene` «Грай ту саму сцену, що й зараз — інакше порівняння буде нечесним.»; `ui.check.recheck` «Я зробив — записати ще раз»; `ui.check.nothing` «Усе гаразд: перевіряти нічого.»; `ui.check.again` «Записати ще раз»; `ui.check.card` «Було / стало»; `ui.check.keep` «Залиш так.»; `ui.check.revert` «Поверни як було.»; `ui.check.ab` «Перевірити точно (A/B)»; `ui.check.trend.better.fps` «Допомогло: середній FPS {from} → {to} ({pct}, поріг {t} %).»; `.better.lows` «Допомогло: 1% low {from} → {to} ({pct}, поріг {t} %) — ривків поменшало.»; `.worse.fps` / `.worse.lows` «Стало гірше: середній FPS / 1% low {from} → {to} ({pct}).»; `.same.fps` / `.same.lows` «Різниці не видно: середній FPS / 1% low {from} → {to} ({pct}) — у межах похибки {t} % одного запису.»; `.unknown.fps` / `.unknown.lows` «Не вдалося порівняти: в одному із записів немає середнього FPS / 1% low.»

- [ ] **Step 1: e2e** `check.spec.ts`: `картка «Огляду» веде в «Діагностику» зі смужкою й 120 с`; `остання перевірка на «Огляді»` (`check_last` фікстура); `«Я зробив» передає check_of, гру, тривалість і сцену` (через `diag-done` + `history-saved` "base1", потім клік → `calls` `diag_start` з `{ exe, seconds: 120, scene, checkOf: "base1" }`); `другий history-saved не міняє «було»` (після повторного запису `diag_start` знову з `checkOf: "base1"`); `fine завершує сценарій`; `insufficient — «Записати ще раз», без «Я зробив»`; `«Було / стало» — краще за 1% low` / `гірше` / `в межах похибки з A/B відкриває «Порівняння»`; `без сценарію «Діагностика» без смужки`; `запис-перевірка з «Історії» показує «Було / стало»`.
- [ ] **Step 2: Run** `cd ui; npx playwright test check` — Expected: FAIL.
- [ ] **Step 3:** реалізувати компоненти, тексти, фікстури.
- [ ] **Step 4: Run** `npm run check; npm run check:i18n; npx playwright test` — Expected: 0 помилок, усі e2e PASS.

### Task 5: документація й підсумкова перевірка

**Files:** `CHANGELOG.md` ([Unreleased] Додано), `docs/HANDOFF.md` (розділ 2y, шапка), `docs/DESIGN.md` (розд. 6 — картка «Огляду», смужка кроків), за потреби README в `crates/history`.

- [ ] **Step 1:** оновити документи.
- [ ] **Step 2: Run** `cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test --workspace; cargo check --workspace --target x86_64-pc-windows-msvc` і `cd ui; npm run check; npm run check:i18n; npx playwright test` — Expected: усе PASS.
