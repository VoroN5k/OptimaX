<script lang="ts">
  // Галерея компонентів «Pro Premium» — лише для розробки (`?gallery`, docs/DESIGN.md етап 3).
  // Показує кожен компонент у всіх станах; дані тут умовні й позначені як приклад. У збірку не
  // потрапляє. Перемикач ефектів і палітри — щоб перевірити режими.
  import Button from "../components/Button.svelte";
  import Card from "../components/Card.svelte";
  import CheckRow from "../components/CheckRow.svelte";
  import Disclosure from "../components/Disclosure.svelte";
  import DualRing from "../components/DualRing.svelte";
  import HeroCard from "../components/HeroCard.svelte";
  import ImpactBadge from "../components/ImpactBadge.svelte";
  import JerkMarker from "../components/JerkMarker.svelte";
  import KpiStrip from "../components/KpiStrip.svelte";
  import Legend from "../components/Legend.svelte";
  import ListRow from "../components/ListRow.svelte";
  import Segmented from "../components/Segmented.svelte";
  import SideTile from "../components/SideTile.svelte";
  import StatusPill from "../components/StatusPill.svelte";
  import SubTabs from "../components/SubTabs.svelte";
  import Toggle from "../components/Toggle.svelte";
  import ToolbarGroup from "../components/ToolbarGroup.svelte";
  import Track from "../components/Track.svelte";
  import VerdictBadge from "../components/VerdictBadge.svelte";

  let effects = $state<"full" | "calm" | "off">("full");
  let palette = $state<"champagne" | "aurora" | "emerald">("champagne");
  let seg = $state<"a" | "b">("a");
  let sub = $state<"x" | "y" | "z">("x");
  let on = $state(true);
  let clicks = $state(0);

  $effect(() => {
    const root = document.documentElement;
    if (effects === "full") delete root.dataset.effects;
    else root.dataset.effects = effects;
    if (palette === "champagne") delete root.dataset.palette;
    else root.dataset.palette = palette;
  });
</script>

<main class="gallery" data-testid="gallery">
  <header class="row">
    <h1>Галерея компонентів</h1>
    <Segmented label="Ефекти" bind:value={effects} options={[{ value: "full", label: "Повні" }, { value: "calm", label: "Спокійні" }, { value: "off", label: "Вимкнені" }]} />
    <Segmented label="Палітра" bind:value={palette} options={[{ value: "champagne", label: "Шампань" }, { value: "aurora", label: "Аврора" }, { value: "emerald", label: "Смарагд" }]} />
  </header>

  <h2 class="micro">Кнопки</h2>
  <div class="row">
    <Button variant="primary" onclick={() => clicks++}>Головна</Button>
    <Button variant="primary" disabled>Неактивна</Button>
    <Button variant="primary" loading>Завантаження</Button>
    <Button>Другорядна</Button>
    <Button disabled>Другорядна неактивна</Button>
    <Button variant="danger" icon="trash">Небезпечна</Button>
    <Button variant="ghost">Прозора</Button>
    <Button small variant="primary">Мала</Button>
    <Button small>Мала другорядна</Button>
    <span class="small dim" data-testid="clicks">кліків: {clicks}</span>
  </div>

  <h2 class="micro">Перемикачі</h2>
  <div class="row">
    <SubTabs label="Підвкладки" value={sub} onselect={(v) => (sub = v)} options={[{ value: "x", label: "Чому лагає?" }, { value: "y", label: "Історія" }, { value: "z", label: "Чому вилітає?" }]} />
    <Segmented label="Вибір" bind:value={seg} options={[{ value: "a", label: "30 с" }, { value: "b", label: "60 с" }]} />
    <Toggle label="Перемикач" bind:checked={on} />
    <Toggle label="Неактивний" checked={false} disabled />
    <ToolbarGroup
      label="Масштаб"
      items={[
        { label: "+", aria: "Наблизити", square: true, onclick: () => {} },
        { label: "−", aria: "Віддалити", square: true, onclick: () => {} },
        { label: "Увесь запис", onclick: () => {} },
      ]}
    />
    <ToolbarGroup
      label="Ривки"
      items={[
        { label: "← Попередній", onclick: () => {}, disabled: true },
        { label: "Найгірший ривок", onclick: () => {}, active: true },
        { label: "Наступний →", onclick: () => {} },
      ]}
    />
  </div>

  <h2 class="micro">Пілюлі й бейджі</h2>
  <div class="row">
    <StatusPill tone="warn" pulse>Помірний вплив</StatusPill>
    <StatusPill tone="bad" pulse>Сильний вплив</StatusPill>
    <StatusPill tone="ok" icon="check">Безпечно</StatusPill>
    <StatusPill tone="neutral">Немає даних</StatusPill>
    <ImpactBadge impact="high" />
    <ImpactBadge impact="moderate" />
    <ImpactBadge impact="info" />
    <VerdictBadge verdict="safe" />
    <VerdictBadge verdict="consequences" />
    <VerdictBadge verdict="protected" />
    <VerdictBadge verdict="unknown" />
  </div>

  <h2 class="micro">Смуга показників</h2>
  <KpiStrip
    label="Підсумок запису (приклад)"
    items={[
      { label: "Середній FPS", value: "142" },
      { label: "1 % low", value: "88", tone: "warn" },
      { label: "0.1 % low", value: null },
      { label: "99-й перцентиль", value: "11.3", unit: "мс" },
    ]}
  >
    {#snippet footer()}<span>Приклад даних галереї</span>{/snippet}
  </KpiStrip>
  <KpiStrip
    label="Порожній стан"
    items={[
      { label: "Середній FPS", value: null },
      { label: "1 % low", value: null },
    ]}
  />

  <h2 class="micro">Головна картка й бічна колонка</h2>
  <div class="bento">
    <div class="span8">
      <HeroCard label="Головна знахідка (приклад)">
        <div class="row"><StatusPill tone="warn" pulse>Помірний вплив</StatusPill><span class="small muted">впевненість 74%</span></div>
        <div class="hero-grid">
          <div>
            <h3 class="hero-title">Процесор<br /><span class="grad">обмежує FPS</span></h3>
            <p class="muted">Приклад тексту знахідки в галереї.</p>
          </div>
          <DualRing outer={{ value: 93, label: "Головний потік зайнятий" }} inner={{ value: 76, label: "GPU завантажений на" }} center="Головний потік" aria="Приклад: головний потік 93 відсотки, GPU 76 відсотків" />
        </div>
        <div>
          <div class="micro">Що можна перевірити</div>
          <ul class="list">
            <CheckRow n={1}>Перший пункт.</CheckRow>
            <CheckRow n={2}>Другий пункт.</CheckRow>
          </ul>
        </div>
        <Disclosure title="Докази" count={2}><p>Вміст згортки.</p></Disclosure>
      </HeroCard>
    </div>
    <div class="span4 col">
      <SideTile title="Бічна знахідка (кнопка)" confidence={74} onclick={() => clicks++}>
        {#snippet pill()}<StatusPill tone="ok" icon="check" small>Безпечно</StatusPill>{/snippet}
      </SideTile>
      <SideTile title="Бічна знахідка (без дії)" confidence={null}>
        {#snippet pill()}<StatusPill tone="neutral" small>Немає даних</StatusPill>{/snippet}
      </SideTile>
      <section class="tile list-tile" aria-label="Розбори">
        <div class="micro pad">Розбори</div>
        <ListRow title="Радар мікрофризів" onclick={() => clicks++}>Ривків: 1</ListRow>
        <ListRow title="Затримка" onclick={() => clicks++} />
      </section>
      <DualRing outer={{ value: null, label: "Немає даних" }} center="Немає запису" aria="Даних немає" />
    </div>
  </div>

  <h2 class="micro">Шкала (доріжки, маркер, легенда)</h2>
  <Card title="Шкала запису (приклад)">
    <div class="timeline">
      <Track label="Кадри" sub="шкала до 28 мс" height={84} aria="Приклад доріжки кадрів">
        <svg viewBox="0 0 100 84" preserveAspectRatio="none"><path d="M0 60 L20 58 L40 61 L45 10 L50 59 L100 60" fill="none" stroke="var(--a1)" stroke-width="1.2" /></svg>
      </Track>
      <Track label="Головний потік" sub="стани потоку" height={24} aria="Приклад станів потоку">
        <div class="bar" style:background="var(--thread-work)"></div>
      </Track>
      <JerkMarker left="calc(148px + (100% - 148px) * 0.45)" title="Ривок" detail="приклад" />
    </div>
    <Legend
      groups={[
        { title: "Метрики", items: [{ kind: "square", color: "--a2", label: "кадр" }, { kind: "square", color: "--warn", label: "ривок" }, { kind: "line", color: "--gpu", label: "GPU" }] },
        { title: "Стани потоків", items: [{ kind: "square", color: "--thread-work", label: "працював" }, { kind: "square", color: "--thread-sleep", label: "спав" }] },
      ]}
    />
  </Card>
</main>

<style>
  .gallery {
    height: 100%;
    overflow-y: auto;
    padding: var(--page-pad);
    display: flex;
    flex-direction: column;
    gap: calc(var(--space) * 5);
    max-width: calc(var(--content-width) + 64px);
    margin: 0 auto;
  }
  /* Плитки з overflow: hidden у flex-колонці інакше стискаються до нуля. */
  .gallery > :global(*) {
    flex-shrink: 0;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(var(--space) * 3);
  }
  .bento {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: var(--gap);
  }
  .span8 {
    grid-column: span 8;
  }
  .span4 {
    grid-column: span 4;
  }
  @media (max-width: 900px) {
    .span8,
    .span4 {
      grid-column: 1 / -1;
    }
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
  }
  .hero-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(300px, 100%), 1fr));
    gap: calc(var(--space) * 7);
    align-items: center;
  }
  .hero-title {
    font-size: var(--fs-hero);
    line-height: var(--lh-hero);
    font-weight: 300;
    letter-spacing: -0.035em;
  }
  .grad {
    font-weight: 600;
    background: var(--text-gradient);
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .list {
    margin: calc(var(--space) * 2) 0 0;
    padding: 0;
  }
  .list-tile {
    padding: calc(var(--space) * 2) 0;
  }
  .pad {
    padding: calc(var(--space) * 3) calc(var(--space) * 5) calc(var(--space) * 2);
  }
  .timeline {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: calc(var(--space) * 3);
    margin-bottom: calc(var(--space) * 5);
    padding-bottom: 30px;
  }
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    top: 4px;
    bottom: 4px;
    border-radius: 4px;
    opacity: 0.8;
  }
</style>
