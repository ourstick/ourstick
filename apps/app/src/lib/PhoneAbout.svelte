<script lang="ts">
  // About OurStick on a phone (ADR 0027): a screen of the app, as a phone
  // has no second window. What a phone needs of the desktop's About: the
  // version, the links, how senders see it, the licences. Not the start at
  // login, the look for new versions (the store updates a phone) or the
  // picture of where to click.
  import { onMount } from "svelte";
  import SendersSeeYou from "./SendersSeeYou.svelte";
  import { aboutView, openAboutLink, type AboutLink, type AboutView } from "./aboutApi";
  import icon from "../../src-tauri/icons/128x128.png";

  let { onBack }: { onBack: () => void } = $props();

  let view = $state<AboutView | null>(null);
  let failure = $state<string | null>(null);
  /** A licence read in place: the app's own page, as built with it. */
  let reading = $state<string | null>(null);

  onMount(() => {
    aboutView().then((about) => (view = about));
  });

  async function open(link: AboutLink) {
    failure = null;
    try {
      await openAboutLink(link);
    } catch (error) {
      failure = String(error);
    }
  }

  /** The page's body, from the app's own files (vite.config.ts makes them);
   * the web view's policy runs no script in it. A licence's text is laid
   * out for 80 columns: on a phone each of its paragraphs flows instead. */
  async function read(page: string) {
    failure = null;
    try {
      const response = await fetch(`/${page}`);
      if (!response.ok) throw new Error(`${page}: ${response.status}`);
      const document = new DOMParser().parseFromString(await response.text(), "text/html");
      for (const pre of Array.from(document.querySelectorAll("pre"))) {
        const flowing = document.createElement("div");
        for (const paragraph of (pre.textContent ?? "").split(/\n\s*\n/)) {
          const text = paragraph.replace(/\s+/g, " ").trim();
          if (!text) continue;
          const line = document.createElement("p");
          line.textContent = text;
          flowing.append(line);
        }
        pre.replaceWith(flowing);
      }
      reading = document.body.innerHTML;
      window.scrollTo(0, 0);
    } catch (error) {
      failure = `Cannot show it: ${error}`;
    }
  }
</script>

{#if reading}
  <section class="reading">
    <button class="back" onclick={() => (reading = null)}>‹ About</button>
    <div class="page">{@html reading}</div>
  </section>
{:else}
  <section class="about">
    <button class="back" onclick={onBack}>‹ Back</button>
    <img src={icon} alt="" width="72" height="72" />
    <h1>OurStick</h1>
    {#if view}<p class="muted">Version {view.version}</p>{/if}
    <p>A USB stick in a link.</p>

    <nav>
      <button class="link" onclick={() => open("website")}>ourstick.com</button>
      <button class="link" onclick={() => open("privacy")}>Privacy</button>
    </nav>
    <!-- The address shown too: without a mail app the link opens nothing. -->
    <p>Report a problem: <button class="link" onclick={() => open("report")}>hi@ourstick.com</button></p>

    <div class="settings"><SendersSeeYou /></div>
    {#if failure}<p class="error">{failure}</p>{/if}

    <footer>
      {#if view?.copyright}<p class="muted small">{view.copyright}</p>{/if}
      <p class="muted small">
        Free software, provided as is, without any warranty (sections 7 and 8 of its licence).
        Keep your own copies of files that matter.
      </p>
      <p class="small">
        <button class="link" onclick={() => read("LICENSE.html")}>OurStick's licence</button>
      </p>
      <p class="small">
        <button class="link" onclick={() => read("THIRD-PARTY-LICENSES.html")}>
          Licences of the libraries in OurStick
        </button>
      </p>
    </footer>
  </section>
{/if}

<style>
  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 2px;
  }
  img {
    margin: 8px 0;
  }
  h1 {
    margin-bottom: 2px;
  }
  nav {
    display: flex;
    gap: 18px;
    margin: 8px 0 4px;
  }
  .settings {
    align-self: stretch;
    text-align: center;
    margin: 14px 0 10px;
  }
  .small {
    font-size: 0.85em;
    margin-bottom: 8px;
  }
  footer {
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
  .back {
    align-self: flex-start;
    border: none;
    background: none;
    padding: 0;
    min-height: 0;
    color: var(--accent);
    cursor: pointer;
    margin-bottom: 8px;
  }
  .reading {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .page {
    overflow-wrap: anywhere;
    font-size: 0.85em;
  }
  .page :global(p) {
    margin: 0 0 10px;
  }
</style>
