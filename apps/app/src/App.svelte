<script lang="ts">
  import { appState, type AppState } from "./lib/api";
  import FirstLaunch from "./lib/FirstLaunch.svelte";
  import Main from "./lib/Main.svelte";
  import Share from "./lib/Share.svelte";

  // The Share popup is the same UI, opened at #share.
  const isShare = location.hash === "#share";

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

{#if isShare}
  <Share />
{:else if failure}
  <main class="card"><p class="error">{failure}</p></main>
{:else if app?.first_launch}
  <FirstLaunch onDone={load} />
{:else if app}
  <Main />
{/if}
