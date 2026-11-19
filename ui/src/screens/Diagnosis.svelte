<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, asCmdError, on } from "../lib/api";
  import type { CheckChoice } from "../lib/bindings/CheckChoice";
  import type { CmdError } from "../lib/bindings/CmdError";
  import type { DiagnosisResult } from "../lib/bindings/DiagnosisResult";
  import type { RecordProgress } from "../lib/bindings/RecordProgress";
  import { sameText, t, tk } from "../lib/i18n";
  import { flashBg, logLocal, ui } from "../lib/state.svelte";
  import Button from "../components/Button.svelte";
  import type { Finding } from "../lib/bindings/Finding";
  import Icon from "../components/Icon.svelte";
  import Card from "../components/Card.svelte";
  import Fold from "../components/Fold.svelte";
  import KpiStrip from "../components/KpiStrip.svelte";
  import CheckSteps from "../components/CheckSteps.svelte";
  import CrashView from "../components/CrashView.svelte";
  import Experiment from "../components/Experiment.svelte";
  import Page from "../components/Page.svelte";
  import GamePicker from "../components/GamePicker.svelte";
  import ProcessPicker from "../components/ProcessPicker.svelte";
  import type { GameEntry } from "../lib/bindings/GameEntry";
  import RecordHistory from "../components/RecordHistory.svelte";
  import RecordStatus from "../components/RecordStatus.svelte";
  import ResultView from "../components/ResultView.svelte";
  import LagWatch from "../components/LagWatch.svelte";
  import ResolutionTest from "../components/ResolutionTest.svelte";
  import StartModePicker from "../components/StartModePicker.svelte";
  import Segmented from "../components/Segmented.svelte";
  import SubTabs from "../components/SubTabs.svelte";
  import StateView from "../components/StateView.svelte";
  import VerdictGuide from "../components/VerdictGuide.svelte";
  import { currentStep, wantsExample } from "../lib/tour.svelte";
  import { EXAMPLE_RESULT } from "../lib/tourExample";
  import { detectGame } from "../lib/detect";

  /** Вкладка: «Чому лагає?» (запис гри), «Щойно лагнуло?» (стеження з буфером) чи «Чому вилітає?». */
  let tab = $state<"lag" | "watch" | "compare" | "history" | "crash">("lag");
  /** Налаштування запису згортаються, коли є результат: зверху має бути відповідь. */
  let setupOpen = $state(true);
  let resOpen = $state(false);
  let resActive = $state(false);
  // Тест роздільності, що вже йде або має підсумок, не ховаємо.
  $effect(() => {
    if (resActive) resOpen = true;
  });
  let exe = $state("");
  /** Гра з бібліотеки; без exe — його визначає бекенд, поки людина запускає гру. */
  let game = $state<GameEntry | null>(null);
  /** Чекаємо, поки гра з бібліотеки з'явиться на екрані. */
  let detecting = $state(false);
  let expPreset = $state<string[] | null>(null);
  let seconds = $state(60);
  let recording = $state(false);
  let progress = $state<RecordProgress | null>(null);
  let result = $state<DiagnosisResult | null>(null);
  let error = $state<CmdError | null>(null);

  onMount(() => {
    // З «Головної» попросили порівняти записи — одразу вкладка історії.
    if (ui.pendingCompare) tab = "history";
    // Сценарій «Перевірити мою гру»: запис гри, 120 с, вибір гри розгорнуто.
    if (ui.check) {
      tab = "lag";
      seconds = ui.check.seconds;
    }
    void api
      .expState()
      .then((v) => {
        if (v) tab = "compare";
      })
      .catch(() => undefined);
    // Повернення на сторінку: показати останній результат і хід запису, якщо він іде.
    void api
      .diagState()
      .then((s) => {
        if (s.recording) {
          recording = true;
          ui.measuring = true;
        }
        // Сценарій лише почався — старий результат не видаємо за його перший крок.
        if (s.last && !result && !(ui.check && !ui.check.baseline)) {
          result = s.last;
          setupOpen = false;
          if (!exe) exe = s.last.diagnosis.game;
        }
      })
      .catch(() => undefined);
    const subs = [
      on("diag-progress", (p) => (progress = p)),
      on("diag-done", (r) => {
        result = r;
        setupOpen = false;
        recording = false;
        afterDetectedRecording(r);
        ui.measuring = false;
        const actionable = r.diagnosis.verdicts.some((v) => v.finding.impact !== "info");
        flashBg(actionable ? "warn" : "ok");
        logLocal("info", tk("ui.diag.done_log", { game: r.diagnosis.game }));
        // Результат — одразу на екрані: смужка кроків сценарію чи верх розповіді. Не блок усередині
        // картки: `scrollIntoView` прокрутив би й її `overflow: hidden` (верх ховався під рамку).
        void tick().then(() =>
          (
            document.querySelector("[data-check-steps]") ??
            document.querySelector("section.story") ??
            document.querySelector("[data-tour=diag-stats]")
          )?.scrollIntoView({
            block: "start",
            behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
          }),
        );
      }),
      on("diag-error", (e) => {
        error = e;
        if (checking && e.kind === "not_found") checkFailed = true;
        recording = false;
        ui.measuring = false;
        flashBg("warn");
      }),
    ];
    return () => subs.forEach((p) => void p.then((u) => u()));
  });

  /** Скасування поточного визначення процесу гри. */
  let cancelDetect: (() => void) | null = null;
  /** Гра, яку зараз чекаємо. */
  let detectingId = $state<string | null>(null);
  /** Exe, які бекенд пропускає в цій сесії (закрились посеред минулого запису). */
  let skipped = $state<string[]>([]);
  /** Чекаємо вже хвилину — підказка про ручний вибір. */
  let slow = $state(false);
  /** Гра, чий щойно визначений exe ще не підтверджено записом до кінця. */
  let justDetected: GameEntry | null = null;

  function stopDetect() {
    cancelDetect?.();
    cancelDetect = null;
    detecting = false;
    detectingId = null;
    slow = false;
  }

  function detectThenStart(g: GameEntry) {
    error = null;
    result = null;
    detecting = true;
    detectingId = g.id;
    skipped = [];
    slow = false;
    cancelDetect = detectGame(g.id, {
      onFound: (found) => {
        cancelDetect = null;
        stopDetect();
        justDetected = g;
        exe = found;
        logLocal("info", tk("ui.lib.detected_log", { game: g.name, exe: found }));
        void start();
      },
      onError: (e) => {
        cancelDetect = null;
        stopDetect();
        error = asCmdError(e);
      },
      onWaiting: (s) => (skipped = s),
      onSlow: () => (slow = true),
    });
  }

  /** «Це все ж гра»: бекенд перестає пропускати exe цієї гри. */
  async function unskip() {
    if (!detectingId) return;
    try {
      await api.libraryForgetExe(detectingId);
      skipped = [];
    } catch (e) {
      error = asCmdError(e);
    }
  }

  // Людина вибрала іншу гру чи перейшла на іншу вкладку — очікування цієї гри більше не потрібне.
  $effect(() => {
    if (detecting && (tab !== "lag" || game?.id !== detectingId)) stopDetect();
  });

  /** Після запису щойно визначеної гри: закрився посеред запису — наступного разу знову чекаємо. */
  function afterDetectedRecording(r: DiagnosisResult) {
    const g = justDetected;
    justDetected = null;
    if (!g) return;
    if (r.end_reason === "game_exited") {
      logLocal("warn", tk("ui.lib.exited_log", { exe: exe, game: g.name }));
      game = g;
      exe = "";
    } else if (r.end_reason === "completed" || r.end_reason === "hotkey") {
      g.exe = exe;
    }
  }

  onMount(() => () => stopDetect());

  function begin() {
    if (game && !game.exe && game.install_dir) detectThenStart(game);
    else void start();
  }

  async function start(checkOf: string | null = null, checkFinding: CheckChoice | null = null) {
    error = null;
    result = null;
    checkFailed = false;
    const c = ui.check;
    if (c && checkOf) {
      c.rechecking = true;
    } else if (c) {
      // Запис «було» сценарію (перший чи заново): умови запам'ятовуються для повторного.
      Object.assign(c, {
        armed: true,
        rechecking: false,
        baseline: null,
        outcome: null,
        afterId: null,
        choice: null,
        mainAdvice: null,
        exe: exe.trim(),
        seconds,
        scene: ui.scene.trim(),
      });
    }
    checking = !!checkOf;
    try {
      await api.diagStart(exe.trim(), seconds, ui.scene.trim() || null, checkOf, checkFinding);
      recording = true;
      ui.measuring = true;
      progress = null;
    } catch (e) {
      error = asCmdError(e);
      if (checkOf && error.kind === "not_found") checkFailed = true;
    }
  }

  /** «Я зробив — записати ще раз»: та сама гра, тривалість і сцена, порівняння з «було». */
  function recheck() {
    const c = ui.check;
    if (!c?.baseline) return;
    exe = c.exe;
    seconds = c.seconds;
    ui.scene = c.scene;
    void start(c.baseline, c.choice);
  }

  /** «Спробувати» пораду: вона стає дією сценарію (крок ④). */
  function tryOption(f: Finding) {
    if (!ui.check) return;
    ui.check.choice = { id: f.id, advice: f.advice };
    // Фокус — на «Я зробив» (кнопка «Спробувати» зникає, клавіатура не має губитися).
    void tick().then(() => {
      document.querySelector(".check-do")?.scrollIntoView({
        block: "nearest",
        behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
      });
      document.querySelector<HTMLButtonElement>("[data-recheck] button")?.focus();
    });
  }

  /** Порада серед дій розповіді результату (головна чи «Інші варіанти»). */
  function inStory(r: DiagnosisResult, c: CheckChoice): boolean {
    const s = r.diagnosis.story;
    if (!s) return false;
    return [...(s.action == null ? [] : [s.action]), ...s.others].some((i) => {
      const f = r.diagnosis.verdicts[i]?.finding;
      return !!f && f.id === c.id && sameText(f.advice, c.advice);
    });
  }

  // Поки показано «було» (зокрема після тесту роздільності, що міг змінити розповідь): запам'ятати
  // його головну пораду для «стало» і скинути вибір, якого в розповіді вже немає.
  $effect(() => {
    const c = ui.check;
    const r = result;
    if (!c || !r || !c.baseline || c.rechecking || isAfter) return;
    const s = r.diagnosis.story;
    const main = s?.action == null ? null : (r.diagnosis.verdicts[s.action]?.finding.advice ?? null);
    if (!sameText(c.mainAdvice, main)) c.mainAdvice = main;
    if (c.choice && !inStory(r, c.choice)) c.choice = null;
  });

  /** Повторний запис «стало» іде (чи це останній запуск) — помилка «було» показується зрозуміло. */
  let checking = false;
  /** Запис «було» недоступний (видалено з «Історії» або без повного запису). */
  let checkFailed = $state(false);

  const scenario = $derived(ui.check);
  /** Показаний результат — збережений повторний запис («стало»), а не «було». */
  const isAfter = $derived(!!scenario?.afterId && ui.lastRecordId === scenario.afterId);
  /** Підсумок «було»: поки показано саме «було» — з нього (тест роздільності міг його уточнити). */
  const baseOutcome = $derived(!isAfter && result && !scenario?.rechecking ? (result.diagnosis.story?.outcome ?? null) : (scenario?.outcome ?? null));
  const checkable = $derived(baseOutcome === "fixable" || baseOutcome === "unexplained");
  /** Поради показаного результату з «Інших варіантів» (для «Усе гаразд»: чи є що спробувати). */
  const hasOptions = $derived(
    !!result?.diagnosis.story?.others.some((i) => !!result?.diagnosis.verdicts[i]?.finding.advice),
  );
  /** Порада, яку перевіряємо: обрана або головна дія «було» (не «стало»). */
  const chosenAdvice = $derived(scenario?.choice?.advice ?? scenario?.mainAdvice ?? null);
  /** Сценарій закінчився: «Усе гаразд» без порад і без вибору. */
  const finished = $derived(
    !!scenario && !!result && !recording && baseOutcome === "fine" && !hasOptions && !scenario.choice,
  );
  /** Є що перевіряти: головна дія чи обрана порада. */
  const hasAction = $derived(checkable || !!scenario?.choice);
  /** «Я зробив» доступне: є «було» і дія. */
  const canRecheck = $derived(!!scenario?.baseline && hasAction && !recording && !detecting);

  function restartCheck() {
    ui.check = { armed: false, exe: "", seconds: 120, scene: "", baseline: null, outcome: null, rechecking: false, afterId: null, choice: null, mainAdvice: null };
    result = null;
    error = null;
    checkFailed = false;
    setupOpen = true;
  }

  // Сценарій стосується однієї гри й вкладки «Чому лагає?»: інша гра чи вкладка — кінець.
  $effect(() => {
    const c = ui.check;
    if (!c) return;
    if (tab !== "lag") ui.check = null;
    else if (c.armed && c.exe && !recording && !detecting && exe.trim() && exe.trim().toLowerCase() !== c.exe.toLowerCase())
      ui.check = null;
  });
  const checkStep = $derived.by(() => {
    if (!scenario) return 0;
    if (detecting || recording) return scenario.baseline ? 5 : 2;
    if (!result) return 1;
    if (isAfter) return 5;
    if (scenario.baseline && hasAction) return 4;
    return 3;
  });

  /** Підсумок тесту роздільності вже показано в його картці — у списку не дублюємо. */
  let resShown = $state(false);

  /** Бекенд уточнив діагноз (тест роздільності) — перечитати. */
  async function reloadResult() {
    try {
      const s = await api.diagState();
      if (s.last) result = s.last;
    } catch {
      /* лишаємо показаний результат */
    }
  }

  /** Знайомство (розділ 3) без власного запису — позначений приклад, нікуди не зберігається. */
  const example = $derived(wantsExample() && !result && !recording);
  const shown = $derived(result ?? (example ? EXAMPLE_RESULT : null));
  let guideOpen = $state(false);

  // Знайомство веде сюди: розділ 2 — розгорнуті налаштування запису, розділ 3 — результат.
  $effect(() => {
    const s = currentStep();
    if (!s || s.screen !== "diagnosis") return;
    tab = "lag";
    setupOpen = s.chapter === 2;
  });

  /** Тест роздільності має сенс, коли вузьке місце — GPU чи CPU (а не ліміт кадрів). */
  /** Наступний крок «Перевірити A/B»: «Порівняння» з грою запису й назвами варіантів. */
  function openCompare(game: string, variants: string[]) {
    exe = game;
    expPreset = variants;
    tab = "compare";
  }

  const modeLabel = $derived(tk(ui.settings.record_start === "hotkey" ? "ui.rec.mode.hotkey" : "ui.rec.mode.auto"));
  const setupSummary = $derived(
    [game?.name || exe.trim() || tk("ui.diag.no_game"), `${seconds} с`, modeLabel, ui.scene.trim()].filter(Boolean).join(" · "),
  );
  const resTestable = $derived(
    !recording && !!result && (result.diagnosis.bottleneck === "gpu" || result.diagnosis.bottleneck === "cpu"),
  );
</script>

<Page
  title={tk(`ui.diag.page.${tab}.title`)}
  subtitle={tk(`ui.diag.page.${tab}.subtitle`)}
>
  {#snippet tabs()}
    <SubTabs
      label={tk("ui.crash.tabs")}
      value={tab}
      onselect={(v) => (tab = v)}
      options={[
        { value: "lag", label: tk("ui.crash.tab_lag") },
        { value: "watch", label: tk("ui.crash.tab_watch") },
        { value: "compare", label: tk("ui.exp.tab") },
        { value: "history", label: tk("ui.history.tab") },
        { value: "crash", label: tk("ui.crash.tab_crash") },
      ]}
    />
    <!-- «Довідка» — у шапці поруч із підрозділами: окремий рядок забирав висоту в результату. -->
    <span class="help" data-tour="diag-help">
      <Button variant="secondary" icon="help" label={tk("ui.guide.open")} onclick={() => (guideOpen = true)} />
    </span>
  {/snippet}
  {#if tab === "crash"}
    <CrashView />
  {:else if tab === "history"}
    <RecordHistory onCompare={openCompare} />
  {:else if tab === "compare"}
    <Experiment bind:game={exe} bind:preset={expPreset} />
  {:else if tab === "watch"}
    <div class="stack">
      <Card>
        <ProcessPicker bind:value={exe} label={tk("ui.diag.game")} />
      </Card>
      <LagWatch game={exe} heading={false} />
    </div>
  {:else}
    <div class="stack">
      {#if scenario}
        <CheckSteps step={checkStep} final={finished} onfinish={() => (ui.check = null)} />
      {/if}
      <Fold title={tk("ui.diag.setup")} summary={setupOpen ? tk("ui.diag.setup_hint") : setupSummary} bind:open={setupOpen}>
        <div class="setup">
          <div data-tour="diag-game"><GamePicker bind:value={exe} bind:game label={tk("ui.diag.game")} /></div>
          <div class="duration" data-tour="diag-duration">
            <span class="small muted">{tk("ui.diag.duration")}</span>
            <Segmented
              label={tk("ui.diag.duration")}
              bind:value={seconds}
              options={[
                { value: 30, label: "30 с" },
                { value: 60, label: "60 с" },
                { value: 120, label: "120 с" },
              ]}
            />
          </div>
        </div>
        <div class="mode" data-tour="diag-start-mode"><StartModePicker /></div>
        <div class="scene">
          <label for="diag-scene" class="small muted">{tk("ui.diag.scene")}</label>
          <input
            id="diag-scene"
            bind:value={ui.scene}
            maxlength="40"
            placeholder={tk("ui.diag.scene_placeholder")}
            autocomplete="off"
          />
          <p class="small dim">{tk("ui.diag.scene_hint")}</p>
        </div>
      </Fold>

      {#if detecting && game}
        <Card>
          <StateView kind="loading" title={tk("ui.lib.detect_title")} text={tk("ui.lib.detect_text", { game: game.name })} />
          {#if skipped.length}
            <div class="detect-note">
              <p class="small muted">{tk("ui.lib.skipped", { exe: skipped.join(", ") })}</p>
              <Button small variant="secondary" onclick={unskip}>{tk("ui.lib.skipped_undo")}</Button>
            </div>
          {/if}
          {#if slow}<p class="small muted detect-note">{tk("ui.lib.detect_slow")}</p>{/if}
        </Card>
      {:else if recording}
        <Card><RecordStatus {progress} game={exe} /></Card>
      {:else if checkFailed}
        <Card>
          <StateView kind="error" title={tk("ui.check.unavailable")} text={error?.message ?? ""}>
            {#snippet action()}<Button onclick={restartCheck}>{tk("ui.check.restart")}</Button>{/snippet}
          </StateView>
        </Card>
      {:else if error?.kind === "no_frames"}
        <Card>
          <StateView kind="error" title={tk("ui.diag.no_frames_title")} text={tk("ui.diag.no_frames_text")} />
          {#if error.message}
            <details class="pm small muted"><summary>{tk("ui.diag.pm_messages")}</summary><pre class="mono">{error.message}</pre></details>
          {/if}
        </Card>
      {:else if error?.kind === "not_started"}
        <Card>
          <StateView kind="error" title={tk("ui.rec.not_started_title")} text={tk(`ui.rec.not_started.${error.message || "wait_timeout"}`)} />
        </Card>
      {:else if error}
        <Card><StateView kind="error" title={tk("ui.common.error")} text={error.message} /></Card>
      {:else if !shown}
        <!-- Без запису — «—», а не правдоподібні числа (docs/DESIGN.md розд. 6, порожній стан). -->
        <KpiStrip
          label={tk("ui.diag.empty_kpi")}
          items={[
            { label: tk("ui.diag.kpi.avg"), value: null },
            { label: tk("ui.diag.kpi.low1"), value: null },
            { label: tk("ui.diag.kpi.low01"), value: null },
            { label: tk("ui.diag.kpi.p99"), value: null },
          ]}
        />
        <Card><StateView kind="empty" title={tk("ui.diag.empty_title")} text={tk("ui.diag.empty_text")} /></Card>
      {/if}

      {#if shown}
        <ResultView
          result={shown}
          {example}
          hideResFindings={resShown}
          onCompare={example ? undefined : openCompare}
          historyId={example ? null : ui.lastRecordId}
          live={!example}
          next={scenario && result && !example ? checkNext : undefined}
          onTry={scenario && result && !example && !isAfter && scenario.baseline ? tryOption : undefined}
          tried={scenario?.choice ?? null}
        >
          {#snippet after()}
            {#if resTestable && result}
              <Fold title={tk("ui.res.title")} summary={tk("ui.res.subtitle")} bind:open={resOpen} keep>
                <ResolutionTest
                  game={result.diagnosis.game}
                  bind:hasReport={resShown}
                  bind:active={resActive}
                  onupdated={reloadResult}
                />
              </Fold>
            {/if}
          {/snippet}
        </ResultView>
      {/if}
    </div>
  {/if}

  {#snippet secondary()}
    {#if recording && tab === "lag"}
      <Button variant="secondary" icon="stop" onclick={() => api.diagCancel()}>{tk("ui.diag.stop")}</Button>
    {:else if detecting && tab === "lag"}
      <Button variant="secondary" icon="close" onclick={stopDetect}>{tk("ui.common.cancel")}</Button>
    {/if}
  {/snippet}
  {#snippet primary()}
    {#if tab === "lag" && canRecheck}
      <span data-recheck><Button variant="primary" icon="play" onclick={recheck}>{tk("ui.check.recheck")}</Button></span>
    {:else if tab === "lag" && !finished}
      <span data-tour="diag-start"><Button variant="primary" icon="play" disabled={!(exe.trim() || game) || recording} loading={recording || detecting} onclick={begin}>
        {tk("ui.diag.start")}
      </Button></span>
    {/if}
  {/snippet}
</Page>

{#snippet checkNext()}
  {#if finished}
    <section class="tile check-done" aria-label={tk("ui.check.done_title")}>
      <span class="done-icon" aria-hidden="true"><Icon name="check" size={20} /></span>
      <div class="done-text">
        <h3>{tk("ui.check.done_title")}</h3>
        <p class="small muted">{tk("ui.check.done_text")}</p>
      </div>
      <div class="done-actions">
        <Button variant="primary" icon="play" onclick={restartCheck}>{tk("ui.check.another")}</Button>
      </div>
    </section>
  {:else if scenario?.baseline && hasAction}
    <section class="tile check-do" aria-label={tk("ui.check.do_title")}>
      <div class="micro">{tk("ui.check.do_title")}</div>
      {#if chosenAdvice}<p class="chosen">{t(chosenAdvice)}</p>{/if}
      <p class="small muted">{tk("ui.check.same_scene")} {tk("ui.check.press_below")}</p>
    </section>
  {:else if baseOutcome === "fine" && hasOptions}
    <section class="tile check-do" aria-label={tk("ui.check.do_title")}>
      <p class="small muted">{tk("ui.check.pick_hint")}</p>
    </section>
  {:else if baseOutcome !== "fine" && !checkable}
    <section class="tile check-do" aria-label={tk("ui.check.do_title")}>
      <p class="small muted">{tk("ui.check.again_text")}</p>
      <div><Button variant="secondary" icon="play" onclick={begin}>{tk("ui.check.again")}</Button></div>
    </section>
  {/if}
{/snippet}

{#if guideOpen}
  <VerdictGuide onclose={() => (guideOpen = false)} />
{/if}

<style>
  .check-do {
    display: grid;
    gap: calc(var(--space) * 3);
    padding: calc(var(--space) * 5);
  }
  .chosen {
    margin: 0;
    color: var(--text);
  }
  .check-done {
    padding: calc(var(--space) * 5);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 4);
  }
  .done-icon {
    display: inline-grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    border: 1px solid var(--ok);
    color: var(--ok);
  }
  .done-text {
    flex: 1;
    min-width: 220px;
    display: grid;
    gap: calc(var(--space) * 1);
  }
  .done-text h3 {
    margin: 0;
    font-size: var(--fs-section);
    font-weight: 400;
  }
  .done-text p {
    margin: 0;
  }
  .done-actions {
    display: flex;
    gap: calc(var(--space) * 2);
  }
  .detect-note {
    display: grid;
    justify-items: center;
    gap: calc(var(--space) * 2);
    margin-top: calc(var(--space) * 2);
    text-align: center;
  }
  .help {
    margin-left: calc(var(--space) * 2);
  }
  .stack {
    display: grid;
    gap: calc(var(--space) * 4);
  }
  .setup {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: calc(var(--space) * 6);
    align-items: start;
  }
  .duration {
    display: grid;
    gap: calc(var(--space) * 2);
  }
  .mode {
    margin-top: calc(var(--space) * 5);
  }
  .scene {
    display: grid;
    gap: calc(var(--space) * 2);
    max-width: 420px;
    margin-top: calc(var(--space) * 5);
  }
  .scene input {
    height: 38px;
    padding: 0 calc(var(--space) * 3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--bg-input);
    font-size: 13px;
  }
  .scene input:focus-visible {
    border-color: var(--accent-from);
  }
  .pm pre {
    white-space: pre-wrap;
  }
</style>
