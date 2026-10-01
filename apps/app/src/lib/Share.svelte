<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { appState, copyLink, copyQrImage, sendByEmail, shareView, type ShareView } from "./api";

  let { stickId }: { stickId: string } = $props();
  let view = $state<ShareView | null>(null);
  let loaded = $state(false);
  let note = $state<string | null>(null);

  async function load() {
    view = await shareView(stickId);
    loaded = true;
  }

  async function act(action: (stickId: string) => Promise<void>, done: string) {
    try {
      await action(stickId);
      note = done;
    } catch (error) {
      note = String(error);
    }
  }

  onMount(() => {
    appState().then((app) => (document.documentElement.dataset.platform = app.platform));
    load();
    // The link appears once the stick is published.
    const stop = listen("sharing-changed", load);
    return () => stop.then((unlisten) => unlisten());
  });
</script>

<main class="card popup">
  <header class="row spread">
    <h1>Share OurStick</h1>
    <button class="close" aria-label="Close" onclick={() => getCurrentWindow().close()}>✕</button>
  </header>
  <p class="muted">Anyone with this link can read the stick.</p>

  {#if view}
    <div class="row">
      <input class="link" readonly value={view.link} onfocus={(event) => event.currentTarget.select()} />
      <button onclick={() => act(copyLink, "Link copied.")}>Copy</button>
    </div>
    <div class="qr">{@html view.qr_svg}</div>
    <div class="row two">
      <button onclick={() => act(copyQrImage, "QR image copied.")}>Copy QR image</button>
      <button onclick={() => act(sendByEmail, "Your mail program should open now.")}>Send by email</button>
    </div>
  {:else if loaded}
    <p>The link is not ready yet: the stick is being published.</p>
  {/if}
  {#if note}<p class="muted note">{note}</p>{/if}
</main>

<style>
  .popup {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .spread {
    justify-content: space-between;
  }
  h1 {
    margin: 0;
  }
  .close {
    border: none;
    background: none;
    font-size: 1.2em;
    padding: 0 4px;
  }
  .link {
    flex: 1;
    min-width: 0;
    font: 0.95em ui-monospace, "SF Mono", "Cascadia Mono", "DejaVu Sans Mono", monospace;
    color: var(--text);
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: var(--button-radius);
    padding: 6px 8px;
    user-select: text;
  }
  .qr {
    align-self: center;
    width: 240px;
    height: 240px;
    border: 1px solid var(--border);
  }
  .qr :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
  .two > button {
    flex: 1;
  }
  .muted {
    color: var(--muted);
    margin: 0;
  }
  .note {
    text-align: center;
  }
</style>
