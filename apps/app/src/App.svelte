<script lang="ts">
  import { appState, type AppState } from "./lib/api";
  import LocalNetworkStep from "./lib/LocalNetworkStep.svelte";
  import KeysMoveStep from "./lib/KeysMoveStep.svelte";
  import Main from "./lib/Main.svelte";
  import Share from "./lib/Share.svelte";
  import Confirm from "./lib/Confirm.svelte";
  import About from "./lib/About.svelte";
  import StillOpen from "./lib/StillOpen.svelte";

  // The Share popup, the Stop sharing / Quit dialog and About are the same
  // UI, opened at #share/<stick id>, #stop/<stick id>, #quit, #still-open
  // (the first ✕, ADR 0028 §6) and #about.
  const [page, pageStick] = location.hash.split("/");

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

{#if page === "#share" && pageStick}
  <Share stickId={pageStick} />
{:else if page === "#stop" && pageStick}
  <Confirm quitting={false} stickId={pageStick} />
{:else if page === "#quit"}
  <Confirm quitting={true} stickId={null} />
{:else if page === "#still-open"}
  <StillOpen />
{:else if page === "#about"}
  <About />
{:else if failure}
  <main class="card"><p class="error">{failure}</p></main>
{:else if app?.move_keys}
  <KeysMoveStep keys={app.move_keys} onDone={load} />
{:else if app?.explain_local_network}
  <LocalNetworkStep onDone={load} />
{:else if app}
  <!-- The first start shows Home with its welcome: it makes nothing. -->
  <Main />
{/if}
