<script lang="ts">
  // ADR 0020: on macOS, before anything reaches the network, so that macOS
  // asks right after "Continue", when the user expects it.
  import { localNetworkExplained } from "./macApi";

  let { onDone }: { onDone: () => void } = $props();
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function go() {
    busy = true;
    failure = null;
    try {
      await localNetworkExplained();
      onDone();
    } catch (error) {
      failure = String(error);
    } finally {
      busy = false;
    }
  }
</script>

<main class="card centred">
  <h1>One question from macOS</h1>
  <p>
    OurStick will ask to find devices on your local network. Allow it: sticks
    then go straight to computers on the same network, much faster. Without
    it, they go through a relay server, slowly.
  </p>
  {#if failure}<p class="error">{failure}</p>{/if}
  <div class="actions">
    <button class="primary" onclick={go} disabled={busy}>Continue</button>
  </div>
</main>
