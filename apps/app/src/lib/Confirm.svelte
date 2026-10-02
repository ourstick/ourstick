<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import { appState, confirmView, quit, stopSharing, type ConfirmView } from "./api";

  let { quitting, stickId }: { quitting: boolean; stickId: string | null } = $props();
  let view = $state<ConfirmView | null>(null);
  let busy = $state(false);
  let failure = $state<string | null>(null);

  onMount(() => {
    appState().then((app) => (document.documentElement.dataset.platform = app.platform));
    confirmView(stickId)
      .then(async (loaded) => {
        view = loaded;
        await fit();
      })
      .catch((error) => (failure = String(error)));
  });

  let card: HTMLElement | undefined = $state();

  /** The window as tall as what it says (long names, two paragraphs). The
   * page is as tall as the window and hides the rest: the card's content is
   * measured, with the page's padding around it. */
  async function fit() {
    await tick();
    if (!card) return;
    const around = card.getBoundingClientRect().top * 2;
    const height = Math.ceil(card.scrollHeight + around + 2);
    const window = getCurrentWindow();
    const now = await window.innerSize();
    const scale = await window.scaleFactor();
    if (height > now.height / scale) {
      await window.setSize(new LogicalSize(now.width / scale, Math.min(height, 640)));
    }
  }

  const close = () => getCurrentWindow().close();

  /** "Photos", "Photos and Films", "Photos, Films and Music". */
  function names(list: string[]): string {
    // Each name isolated (FIRST STRONG ISOLATE … POP DIRECTIONAL ISOLATE): a
    // sender's writing direction never spills over the sentence.
    const [open, close] = [String.fromCodePoint(0x2068), String.fromCodePoint(0x2069)];
    const quoted = list.map((name) => `“${open}${name}${close}”`);
    return quoted.length < 2 ? (quoted[0] ?? "") : `${quoted.slice(0, -1).join(", ")} and ${quoted.at(-1)}`;
  }

  async function stop() {
    busy = true;
    failure = null;
    try {
      if (quitting) {
        // Sharing stops on the way out.
        await quit();
      } else if (stickId) {
        await stopSharing(stickId);
        await close();
      }
    } catch (error) {
      failure = String(error);
      busy = false;
    }
  }
</script>

<main class="card dialog" bind:this={card}>
  {#if failure}
    <p class="error">{failure}</p>
    {#if !view}
      <div class="actions"><button onclick={close}>Close</button></div>
    {/if}
  {/if}
  <!-- Nothing until the app says whether it shares: never the wrong words. -->
  {#if view && quitting}
    <h1>Quit OurStick?</h1>
    {#if view.shared.length > 0}
      <p>
        Sharing {names(view.shared)} stops: anyone with {view.shared.length === 1 ? "its sharing link" : "their sharing links"} will no
        longer be able to download.
      </p>
      {#if view.downloading.length > 0}
        <p><strong>Someone is still downloading from {names(view.downloading)}.</strong></p>
      {/if}
    {:else}
      <p>No stick is being shared.</p>
    {/if}
    {#if view.receiving.length > 0}
      <p>Getting {names(view.receiving)} stops; it goes on where it was next time.</p>
    {/if}
    <div class="actions">
      <button onclick={close} disabled={busy}>
        {view.sharing || view.receiving.length > 0 ? "Don't quit" : "Cancel"}
      </button>
      <button class={view.sharing ? "danger" : "primary"} onclick={stop} disabled={busy}>Quit</button>
    </div>
  {:else if view}
    <h1>Stop sharing {view.name ? `“${view.name}”` : "this stick"}?</h1>
    <p>Anyone with the sharing link will no longer be able to download.</p>
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
