<script lang="ts">
  import { appState, type AppState } from "./lib/api";
  import FirstLaunch from "./lib/FirstLaunch.svelte";
  import Main from "./lib/Main.svelte";
  import Share from "./lib/Share.svelte";
  import Confirm from "./lib/Confirm.svelte";

  // The Share popup and the Stop sharing / Quit dialog are the same UI,
  // opened at #share/<stick id>, #stop/<stick id> and #quit.
  const [page, pageStick] = location.hash.split("/");

  let app = $state<AppState | null>(null);
  /** First launch, "I got a sharing link": straight to adding it. */
  let receiving = $state(false);
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

{#if page === "#share" && pageStick}
  <Share stickId={pageStick} />
{:else if page === "#stop" && pageStick}
  <Confirm quitting={false} stickId={pageStick} />
{:else if page === "#quit"}
  <Confirm quitting={true} stickId={null} />
{:else if failure}
  <main class="card"><p class="error">{failure}</p></main>
{:else if app?.first_launch && !receiving}
  <FirstLaunch onDone={load} onReceive={() => (receiving = true)} />
{:else if app}
  <Main startAdding={receiving} />
{/if}
