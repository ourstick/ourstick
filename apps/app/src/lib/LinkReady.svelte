<script lang="ts">
  // Send, step 2: the link (gui-spec, Send). The link itself never reaches
  // the window: it is shown hidden but for its start, and `Copy link` puts
  // it on the clipboard from the app's side.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { copyLink, onPhone, shareLink, stick as loadStick, type Stick } from "./api";

  let {
    stickId,
    onBack,
    onSeeWho,
  }: { stickId: string; onBack: () => void; onSeeWho: () => void } = $props();

  let current = $state<Stick | null>(null);
  /** A phone sends the link through its share sheet (ADR 0029). */
  const phone = onPhone();
  let copied = $state(false);
  let failure = $state<string | null>(null);

  async function refresh() {
    try {
      current = await loadStick(stickId);
    } catch (error) {
      failure = String(error);
    }
  }

  async function share() {
    failure = null;
    try {
      await shareLink(stickId);
    } catch (error) {
      failure = String(error);
    }
  }

  async function copy() {
    failure = null;
    try {
      await copyLink(stickId);
      copied = true;
    } catch (error) {
      failure = String(error);
    }
  }

  onMount(() => {
    refresh();
    const stops = [listen("sharing-changed", refresh), listen("stick-changed", refresh)];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  });

  let ready = $derived(current?.sharing.has_link ?? false);
</script>

<section class="view">
  <button class="back" onclick={onBack}>‹ Back</button>
  {#if current}
    <h1>“<bdi>{current.name}</bdi>” {ready ? "is ready to send" : "is getting ready…"}</h1>
    <div class="link">
      <span class="hidden-link" aria-label="The link, hidden">ourstick.com/s/#•••</span>
      {#if phone}
        <button class="primary" onclick={share} disabled={!ready}>Send the link</button>
      {/if}
      <button class:primary={!phone} onclick={copy} disabled={!ready}>{copied ? "Copied ✓" : "Copy link"}</button>
    </div>
    <p class="warning trust">
      Anyone who gets this link can take everything in it, also the files you add later. Send it only to people you trust.
    </p>
    {#if current.sharing.preparing}
      <p class="muted">People with the link can start already: the rest follows by itself.</p>
    {/if}
    <ol class="steps">
      {#if phone}
        <li>Send it in a message: Send the link opens your phone’s list of apps.</li>
        <li>They need OurStick too. When they tap the link, the page shows them how to get it.</li>
        <li>
          Then come back here and <b>keep OurStick open until they have it</b>, with the phone on charge for a big one:
          a phone stops sending once you leave OurStick.
        </li>
      {:else}
        <li>Paste it in an email or a message.</li>
        <li>They need OurStick too. When they click the link, the page shows them how to get it.</li>
        <li>Keep this computer on and OurStick open. They get what you add or change, until you stop sending.</li>
      {/if}
    </ol>
    <button class="see-who" onclick={onSeeWho}>See who gets it ›</button>
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
</section>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
    overflow-y: auto;
  }
  .back,
  .see-who {
    align-self: flex-start;
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
  }
  h1 {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .link {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .hidden-link {
    flex: 1 1 auto;
    font-family: ui-monospace, monospace;
    color: var(--muted);
  }
  .trust {
    margin: 0;
  }
  .steps {
    margin: 0;
    padding-left: 22px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
</style>
