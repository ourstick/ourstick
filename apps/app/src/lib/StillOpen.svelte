<script lang="ts">
  // "OurStick is still open" (ADR 0028 §6): at the first ✕ only. No words
  // like Dock, taskbar or tray: "its picture", "near the clock".
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { openDialog } from "./api";
  import ScreenPicture from "./ScreenPicture.svelte";

  let view = $state<{ platform: string; tray: boolean } | null>(null);
  let failure = $state<string | null>(null);

  onMount(() => {
    invoke<{ platform: string; tray: boolean }>("still_open_view")
      .then((loaded) => {
        view = loaded;
        document.documentElement.dataset.platform = loaded.platform;
      })
      .catch((error) => (failure = String(error)));
  });

  async function ok() {
    try {
      await invoke("still_open_ok");
      await getCurrentWindow().close();
    } catch (error) {
      failure = String(error);
    }
  }

  /** `Quit OurStick`: the Quit dialog, which asks first; told stays unset. */
  async function quit() {
    try {
      await openDialog(true, null);
    } catch (error) {
      failure = String(error);
    }
  }

  let gnome = $derived(view?.platform === "linux" && !view.tray);
</script>

<main class="card still-open">
  <h1>OurStick is still open</h1>
  {#if view}
    <ScreenPicture platform={view.platform} tray={view.tray} />
    {#if gnome}
      <p>To open it again, press the key with the Windows logo <b>⊞</b>, then click <b>OurStick</b>.</p>
    {:else}
      <p>
        To open it again, click its picture {view.platform === "linux" ? "on the left" : "at the bottom"} of the screen.
      </p>
      {#if view.platform === "windows"}
        <p>It is also near the clock. If you do not see it there, click <b>^</b>.</p>
      {:else if view.platform === "macos"}
        <p>It is also at the top of the screen, near the clock.</p>
      {:else}
        <p>It is also at the top right, near the clock.</p>
      {/if}
    {/if}
  {/if}
  <p class="muted">What you send keeps going while it is open. If you quit, sending and receiving stop until you open it again.</p>
  {#if failure}<p class="error">{failure}</p>{/if}
  <div class="actions">
    <button onclick={quit}>Quit OurStick</button>
    <!-- svelte-ignore a11y_autofocus -->
    <button class="primary" autofocus onclick={ok}>OK</button>
  </div>
</main>

<style>
  .still-open {
    gap: 12px;
  }
  h1 {
    margin: 0;
    font-size: 1.35em;
  }
  p {
    margin: 0;
    font-size: 1.05em;
    line-height: 1.45;
  }
  .muted {
    font-size: 0.95em;
  }
  .actions {
    margin-top: auto;
  }
</style>
