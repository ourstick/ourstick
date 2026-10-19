<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { listen } from "@tauri-apps/api/event";
  import { appState, confirmView, minimizeMain, quit, stopSharing, type ConfirmView } from "./api";
  import { setStartAtLogin, startAtLogin } from "./aboutApi";

  /** `closing`: Quit from the main window's ✕, with Minimize as the other choice. */
  let {
    quitting,
    closing = false,
    stickId,
  }: { quitting: boolean; closing?: boolean; stickId: string | null } = $props();
  let view = $state<ConfirmView | null>(null);
  let busy = $state(false);
  let failure = $state<string | null>(null);

  /** Start at login (ADR 0024) on Quit: the moment one thinks of it. The
   * same choice as the tray's and About's, changed at once. */
  let login = $state<boolean | null>(null);

  async function readLogin() {
    login = (await startAtLogin()).on;
  }

  async function changeLogin(on: boolean) {
    failure = null;
    try {
      await setStartAtLogin(on);
    } catch (error) {
      failure = String(error);
    }
    await readLogin();
  }

  onMount(() => {
    appState().then((app) => (document.documentElement.dataset.platform = app.platform));
    confirmView(stickId)
      .then((loaded) => (view = loaded))
      .catch((error) => (failure = String(error)));
    if (!quitting) return;
    readLogin().catch((error) => (failure = String(error)));
    const stop = listen("start-at-login-changed", readLogin);
    return () => stop.then((unlisten) => unlisten());
  });

  const close = () => getCurrentWindow().close();

  async function minimize() {
    try {
      await minimizeMain();
      await close();
    } catch (error) {
      failure = String(error);
    }
  }

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

<main class="card dialog">
  {#if failure}
    <p class="error">{failure}</p>
    {#if !view}
      <div class="actions"><button onclick={close}>Close</button></div>
    {/if}
  {/if}
  <!-- Nothing until the app says whether it shares: never the wrong words. -->
  {#if view && quitting}
    <h1>Quit OurStick?</h1>
    <!-- What it says scrolls if it is long; the buttons always show. -->
    <div class="said">
    {#if view.shared.length > 0}
      <p>
        Sharing {names(view.shared)} stops: anyone with {view.shared.length === 1 ? "its sharing link" : "their sharing links"}
        <strong>will no longer be able to download.</strong>
      </p>
      {#if view.downloading.length > 0}
        <p><strong>Someone is still downloading from {names(view.downloading)}.</strong></p>
      {/if}
    {:else}
      <p>No stick is being shared.</p>
    {/if}
    {#if view.receiving.length > 0}
      <p>Getting {names(view.receiving)} stops; it goes on where it was next time.</p>
    {:else if view.following}
      <p>The sticks you receive get nothing new until OurStick opens again.</p>
    {/if}
    </div>
    {#if busy}
      <p class="muted">Quitting… please wait.</p>
    {/if}
    {#if login !== null}
      <label class="row login">
        <input type="checkbox" checked={login} disabled={busy} onchange={(event) => changeLogin(event.currentTarget.checked)} />
        Start OurStick when I log in, so that my sticks are shared after a restart
      </label>
    {/if}
    <div class="actions">
      {#if closing}
        <button onclick={minimize} disabled={busy}>Minimize</button>
      {:else}
        <button onclick={close} disabled={busy}>
          {view.sharing || view.following ? "Don't quit" : "Cancel"}
        </button>
      {/if}
      <button
        class={view.sharing ? "danger" : "primary"}
        onclick={stop}
        disabled={busy}>Quit</button>
    </div>
  {:else if view}
    <h1>Stop sending {view.name ? `“${view.name}”` : "it"}?</h1>
    <p>The link stops working until you start again. Your files stay where they are.</p>
    {#if view.readers > 0}
      <p><strong>Someone is getting it right now.</strong></p>
    {/if}
    {#if busy}<p class="muted">Stopping… please wait.</p>{/if}
    <div class="actions">
      <button class="primary" onclick={close} disabled={busy}>Keep sending</button>
      <button class="danger" onclick={stop} disabled={busy}>Stop</button>
    </div>
  {/if}
</main>

<style>
  .dialog h1 {
    font-size: 1.2em;
  }
  .said {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }
  .dialog .actions {
    flex-shrink: 0;
  }
  .login {
    flex-shrink: 0;
    align-items: flex-start;
    gap: 8px;
  }
  .login input {
    margin-top: 3px;
  }
</style>
