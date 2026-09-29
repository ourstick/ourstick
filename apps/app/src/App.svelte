<script lang="ts">
  import { appState, type AppState } from "./lib/api";
  import FirstLaunch from "./lib/FirstLaunch.svelte";
  import Main from "./lib/Main.svelte";

  let app = $state<AppState | null>(null);
  let failure = $state<string | null>(null);

  async function load() {
    try {
      app = await appState();
      // Once, at start: the theme follows the platform (docs/gui-spec.md).
      document.documentElement.dataset.platform = app.platform;
    } catch (error) {
      failure = String(error);
    }
  }
  load();
</script>

{#if failure}
  <main class="card"><p class="error">{failure}</p></main>
{:else if app?.first_launch}
  <FirstLaunch onDone={load} />
{:else if app}
  <Main />
{/if}
