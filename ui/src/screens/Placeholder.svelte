<script lang="ts">
  import { tk } from "../lib/i18n";
  import { ui } from "../lib/state.svelte";
  import Card from "../components/Card.svelte";
  import Page from "../components/Page.svelte";
  import StateView from "../components/StateView.svelte";

  // Чесна заглушка: розділ є в навігації, але функцій ще немає — жодних фейкових кнопок.
</script>

<Page title={tk(`ui.nav.${ui.screen}`)} subtitle={tk(`ui.placeholder.${ui.screen}`)}>
  <Card>
    <StateView kind="empty" title={tk("ui.placeholder.title")} text={tk("ui.placeholder.text")} />
  </Card>
</Page>
