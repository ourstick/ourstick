<script lang="ts">
  // About OurStick (ADR 0024): what it is, its version and licences, the
  // links, and the start at login check (the same choice as the tray's).
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { appState } from "./api";
  import {
    aboutView,
    openAboutLink,
    setStartAtLogin,
    startAtLogin,
    type AboutLink,
    type AboutView,
  } from "./aboutApi";
  import icon from "../../src-tauri/icons/128x128.png";

  let view = $state<AboutView | null>(null);
  let login = $state<boolean | null>(null);
  let macos = $state(false);
  let failure = $state<string | null>(null);

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

  async function open(link: AboutLink) {
    failure = null;
    try {
      await openAboutLink(link);
    } catch (error) {
      failure = String(error);
    }
  }

  onMount(() => {
    appState().then((app) => {
      document.documentElement.dataset.platform = app.platform;
      macos = app.platform === "macos";
    });
    aboutView().then((about) => (view = about));
    readLogin();
    // The tray's check changes it too.
    const stop = listen("start-at-login-changed", readLogin);
    return () => stop.then((unlisten) => unlisten());
  });
</script>

<main class="card about">
  <img src={icon} alt="" width="80" height="80" />
  <h1>OurStick</h1>
  {#if view}<p class="muted">Version {view.version}</p>{/if}
  <p>A USB stick in a link.</p>

  <nav>
    <button class="link" onclick={() => open("website")}>ourstick.com</button>
    <button class="link" onclick={() => open("privacy")}>Privacy</button>
    <button class="link" onclick={() => open("report")}>Report a problem</button>
  </nav>

  {#if login !== null}
    <label class="row login">
      <input type="checkbox" checked={login} onchange={(event) => changeLogin(event.currentTarget.checked)} />
      Start OurStick when I log in, so that my sticks are shared after a restart
    </label>
    {#if macos}
      <p class="muted small">macOS also lists it in System Settings → General → Login Items.</p>
    {/if}
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}

  <footer>
    {#if view?.copyright}<p class="muted small">{view.copyright}</p>{/if}
    <p class="small">
      <button class="link" onclick={() => open("licence")}>OurStick's licence</button>
      ·
      <button class="link" onclick={() => open("libraries")}>Licences of the libraries in OurStick</button>
    </p>
    <div class="actions">
      <button onclick={() => getCurrentWindow().close()}>Close</button>
    </div>
  </footer>
</main>

<style>
  .about {
    align-items: center;
    text-align: center;
  }
  img {
    margin: 4px 0 8px;
  }
  h1 {
    margin-bottom: 2px;
  }
  nav {
    display: flex;
    gap: 14px;
    margin: 4px 0 14px;
  }
  .login {
    align-items: flex-start;
    text-align: left;
    gap: 8px;
    margin-bottom: 6px;
  }
  .login input {
    margin-top: 3px;
  }
  .small {
    font-size: 0.85em;
    margin-bottom: 6px;
  }
  footer {
    margin-top: auto;
    width: 100%;
  }
  .link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    min-height: 0;
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
