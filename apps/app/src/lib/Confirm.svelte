<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { appState, confirmView, quit, stopSharing, type ConfirmView } from "./api";

  let { quitting, stickId }: { quitting: boolean; stickId: string | null } = $props();
  let view = $state<ConfirmView | null>(null);
  let busy = $state(false);

  onMount(() => {
    appState().then((app) => (document.documentElement.dataset.platform = app.platform));
    confirmView(stickId).then((loaded) => (view = loaded));
  });

  const close = () => getCurrentWindow().close();

  async function stop() {
    busy = true;
    if (quitting) {
      // Sharing stops on the way out.
      await quit();
    } else if (stickId) {
      await stopSharing(stickId);
      await close();
    }
  }
</script>

<main class="card dialog">
  <!-- Nothing until the app says whether it shares: never the wrong words. -->
  {#if view && quitting}
    <h1>Quit OurStick?</h1>
    {#if view.sharing}
      <p>Sharing stops: anyone with the link will no longer be able to download.</p>
      {#if view.readers > 0}
        <p><strong>Someone is still downloading.</strong></p>
      {/if}
    {:else}
      <p>The stick is not being shared.</p>
    {/if}
    <div class="actions">
      <button onclick={close} disabled={busy}>{view.sharing ? "Keep sharing" : "Cancel"}</button>
      <button class={view.sharing ? "danger" : "primary"} onclick={stop} disabled={busy}>Quit</button>
    </div>
  {:else if view}
    <h1>Stop sharing OurStick?</h1>
    <p>Anyone with the link will no longer be able to download.</p>
    {#if view.readers > 0}
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
