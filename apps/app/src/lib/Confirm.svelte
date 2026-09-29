<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { appState, confirmView, quit, stopSharing, type ConfirmView } from "./api";

  let { quitting }: { quitting: boolean } = $props();
  let view = $state<ConfirmView | null>(null);
  let busy = $state(false);

  onMount(() => {
    appState().then((app) => (document.documentElement.dataset.platform = app.platform));
    confirmView().then((loaded) => (view = loaded));
  });

  const close = () => getCurrentWindow().close();

  async function stop() {
    busy = true;
    if (quitting) {
      // Sharing stops on the way out.
      await quit();
    } else {
      await stopSharing();
      await close();
    }
  }
</script>

<main class="card dialog">
  {#if quitting && view && !view.sharing}
    <h1>Quit OurStick?</h1>
    <p>The stick is not being shared.</p>
    <div class="actions">
      <button onclick={close} disabled={busy}>Cancel</button>
      <button class="primary" onclick={stop} disabled={busy}>Quit</button>
    </div>
  {:else}
    <h1>Stop sharing OurStick?</h1>
    <p>Anyone with the link will no longer be able to download.</p>
    {#if view && view.readers > 0}
      <p><strong>Someone is still downloading.</strong></p>
    {/if}
    <div class="actions">
      <button onclick={close} disabled={busy}>Keep sharing</button>
      <button class="danger" onclick={stop} disabled={busy}>Stop</button>
    </div>
  {/if}
</main>

<style>
  .dialog h1 {
    font-size: 1.2em;
  }
</style>
