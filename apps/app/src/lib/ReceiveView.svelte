<script lang="ts">
  // Receive (gui-spec, ADR 0028 §4): the link found on the clipboard when
  // Receive was pressed (and only then), else pasted; the sender's computer
  // found; what it brings, where it goes, and Download. The link itself is
  // never shown, logged or put into a message.
  import { onMount } from "svelte";
  import { linkOnClipboard } from "./macApi";
  import {
    addReceived,
    chooseFolder,
    previewInside,
    previewLink,
    previewNeeds,
    waitForLink,
    type PreviewEntry,
    type PreviewView,
  } from "./api";
  import { holdsWords, lastParts, size } from "./format";
  import ChoiceTree, { type ChoiceRow } from "./ChoiceTree.svelte";
  import SavedAsNote from "./SavedAsNote.svelte";

  let {
    onAdded,
    onShow,
  }: {
    /** Downloading (or waited for): its view. */
    onAdded: (stickId: string) => void;
    /** A link known here: the view of what one sends, or one receives. */
    onShow: (kind: "sending" | "received", stickId: string) => void;
  } = $props();

  type Phase = "starting" | "paste" | "looking" | "seen" | "choose" | "away" | "known";
  let phase = $state<Phase>("starting");
  /** The link came from the clipboard. */
  let found = $state(false);
  let text = $state("");
  /** The link being looked at, or seen: Download adds exactly that one. */
  let looked = "";
  let parent = $state<string | null>(null);
  let preview = $state<PreviewView | null>(null);
  let known = $state<{ known: "own" | "received" | "waiting"; id: string; name: string } | null>(null);
  /** The paths left out (ADR 0012 and its Amendment 1), exactly as named. */
  let leftOut = $state<string[]>([]);
  let none = $state(false);
  /** What would be written with this choice, as the core counts it. */
  let needs = $state(0);
  let adding = $state(false);
  let failure = $state<string | null>(null);
  /** A Cancel while looking drops that answer. */
  let looks = 0;

  async function look(link: string) {
    const mine = ++looks;
    phase = "looking";
    failure = null;
    try {
      const seen = await previewLink(link, parent);
      if (mine !== looks) return;
      if (seen.kind === "sender_away") {
        looked = link;
        phase = "away";
      } else if (seen.kind === "known") {
        known = seen;
        phase = "known";
      } else {
        if (link !== looked) leftOut = [];
        looked = link;
        preview = seen.preview;
        phase = "seen";
      }
    } catch (error) {
      if (mine !== looks) return;
      failure = String(error);
      phase = preview && link === looked ? "seen" : "paste";
    }
  }

  function cancelLook() {
    looks++;
    phase = preview ? "seen" : "paste";
  }

  function open(event: SubmitEvent) {
    event.preventDefault();
    found = false;
    preview = null;
    if (text.trim()) void look(text.trim());
  }

  function another() {
    found = false;
    preview = null;
    text = "";
    looked = "";
    failure = null;
    phase = "paste";
  }

  // Read when Receive was just pressed, and again each time the window
  // comes back while "Paste the link you got" waits with its field empty
  // (ADR 0028 §4, amended): the user went to copy it. Only an OurStick link
  // is taken; a read that fails or is refused is the same as nothing there.
  async function fromClipboard() {
    const copied = await linkOnClipboard().catch(() => null);
    if (copied) {
      found = true;
      await look(copied);
    } else if (phase === "starting") {
      phase = "paste";
    }
  }

  onMount(() => {
    void fromClipboard();
    const back = () => {
      if (phase === "paste" && !text.trim() && document.visibilityState === "visible") void fromClipboard();
    };
    window.addEventListener("focus", back);
    return () => window.removeEventListener("focus", back);
  });

  async function change() {
    const picked = await chooseFolder().catch((error) => {
      failure = String(error);
      return null;
    });
    if (picked && looked) {
      parent = picked;
      const was = phase;
      await look(looked);
      if (was === "choose" && phase === "seen") phase = "choose";
    }
  }

  async function waitForIt() {
    adding = true;
    failure = null;
    try {
      onAdded(await waitForLink(looked));
    } catch (error) {
      failure = String(error);
    } finally {
      adding = false;
    }
  }

  async function download() {
    if (!preview || adding) return;
    adding = true;
    failure = null;
    try {
      onAdded(await addReceived(looked, preview.folder, preview.name, leftOut));
    } catch (error) {
      failure = String(error);
    } finally {
      adding = false;
    }
  }

  /** Largest first: size is why the list is opened. */
  let entries = $derived<ChoiceRow[]>(preview ? [...preview.entries].sort((a, b) => b.bytes - a.bytes) : []);

  async function loadInside(folder: string): Promise<ChoiceRow[]> {
    return (await previewInside(folder)).sort((a: PreviewEntry, b: PreviewEntry) => b.bytes - a.bytes);
  }

  // What the choice needs, asked of the core whenever it or the preview
  // changes; only the latest answer counts.
  let askedNeeds = 0;
  $effect(() => {
    if (!preview) return;
    const set = [...leftOut];
    const mine = ++askedNeeds;
    previewNeeds(set)
      .then((answer) => {
        if (mine === askedNeeds) needs = answer;
      })
      .catch((error) => (failure = String(error)));
  });
  let fits = $derived(preview !== null && needs <= preview.left);

  let holds = $derived(preview ? holdsWords(preview.entries) : "");
  let goesTo = $derived(preview ? lastParts(preview.folder) : "");
</script>

<section class="receive">
  {#if phase === "starting"}
    <p class="muted">Looking for a link you copied…</p>
  {:else if phase === "paste"}
    <h1>Paste the link you got</h1>
    <ol class="steps">
      <li>In the email or message you got, copy the whole link. It starts with <b>ourstick.com/s/</b></li>
      <li>Come back here: OurStick finds it by itself. Or paste it below.</li>
    </ol>
    <form onsubmit={open}>
      <label for="the-link">The link</label>
      <div class="row">
        <!-- svelte-ignore a11y_autofocus -->
        <input
          id="the-link"
          bind:value={text}
          placeholder="https://ourstick.com/s/#…"
          autofocus
          spellcheck="false"
          autocomplete="off"
        />
        <button class="primary" type="submit" disabled={!text.trim()}>Open</button>
      </div>
    </form>
    <p class="muted">Clicked the link and a web page opened? Press “Copy the link” on that page, then come back here.</p>
  {:else if phase === "looking"}
    {#if found}<p class="found">✓ We found a link you copied</p>{/if}
    <h1>Someone sent you files</h1>
    <div class="box">
      <strong>Finding the sender’s computer…</strong>
      <span class="muted">This takes up to half a minute.</span>
      <button onclick={cancelLook}>Cancel</button>
    </div>
  {:else if phase === "away"}
    <h1>The sender's computer is off right now</h1>
    <p>Or OurStick is closed there. You can wait: it comes by itself, all of it, once their computer is on.</p>
    <div class="actions">
      <button onclick={another} disabled={adding}>Cancel</button>
      <button onclick={() => look(looked)} disabled={adding}>Try again now</button>
      <button class="primary" onclick={waitForIt} disabled={adding}>Wait for it</button>
    </div>
  {:else if phase === "known" && known}
    {#if known.known === "own"}
      <h1>This is the link to “<bdi>{known.name}</bdi>”, which you send yourself.</h1>
    {:else if known.known === "received"}
      <h1>You already get “<bdi>{known.name}</bdi>” on this computer.</h1>
    {:else}
      <h1>You already wait for this link.</h1>
    {/if}
    <div class="actions">
      <button onclick={another}>Paste another link</button>
      <button class="primary" onclick={() => known && onShow(known.known === "own" ? "sending" : "received", known.id)}
        >Show it</button
      >
    </div>
  {:else if preview && phase === "seen"}
    <div class="card-head">
      {#if found}<p class="found">✓ We found a link you copied</p>{/if}
      <h1><bdi>{preview.name}</bdi></h1>
      <p class="muted">
        {holds} · {size(needs)}{fits ? " · it fits on this computer" : ""}
      </p>
    </div>
    {#if !fits}
      <div class="box error-box">
        <strong class="error">It does not fit: only {size(preview.left)} are free.</strong>
        <span>Take only some of it, save it on another disk, or make some room.</span>
        <div class="row">
          <button onclick={() => (phase = "choose")}>Only some of it…</button>
          <button onclick={change}>Another place…</button>
        </div>
      </div>
    {/if}
    {#if preview.partial}
      <p class="muted">The sender is still counting, so more is coming. You can start now: the rest follows.</p>
    {/if}
    <div class="row spread where">
      <span>It goes to <b><bdi>{goesTo}</bdi></b></span>
      <button onclick={change} disabled={adding}>Change…</button>
    </div>
    {#if preview.earlier_elsewhere}
      <p class="muted">
        You received it before, in <bdi>{preview.earlier_elsewhere}</bdi>. The files there stay there; it starts again
        in the new folder.
      </p>
    {/if}
    <SavedAsNote list={preview.saved_as} coming />
    <button class="link" onclick={() => (phase = "choose")}>I want only some of it…</button>
    {#if failure}<p class="error">{failure}</p>{/if}
    <div class="row spread bottom">
      <button class="link" onclick={another}>Not this one? Paste another link</button>
      <button class="primary" onclick={download} disabled={adding || !fits}>Download</button>
    </div>
  {:else if preview && phase === "choose"}
    <h1>What do you want from “<bdi>{preview.name}</bdi>”?</h1>
    <div class="row">
      <button onclick={() => (leftOut = [])} disabled={adding || leftOut.length === 0}>All</button>
      <button onclick={() => (leftOut = entries.map((entry) => entry.name))} disabled={adding || none}>None</button>
    </div>
    <div class="tree">
      <ChoiceTree rows={entries} bind:leftOut bind:none {loadInside} disabled={adding} onFailure={(why) => (failure = why)} />
    </div>
    {#if none}
      <p class="error">Tick at least one thing.</p>
    {:else}
      <p class:error={!fits} class:muted={fits}>
        {size(needs)} · {fits ? "it fits" : `only ${size(preview.left)} are free`}
      </p>
    {/if}
    <div class="row spread">
      <span>To <b><bdi>{goesTo}</bdi></b></span>
      <button onclick={change} disabled={adding}>Change…</button>
    </div>
    {#if failure}<p class="error">{failure}</p>{/if}
    <div class="row spread bottom">
      <button class="link" onclick={() => (phase = "seen")}>‹ All of it</button>
      <button class="primary" onclick={download} disabled={adding || !fits || none}>Download</button>
    </div>
  {/if}
  {#if failure && (phase === "paste" || phase === "away" || phase === "known")}<p class="error">{failure}</p>{/if}
  {#if adding}<p class="muted">Starting… please wait.</p>{/if}
</section>

<style>
  .receive {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
    overflow-y: auto;
  }
  h1 {
    margin: 0;
    overflow-wrap: anywhere;
  }
  p {
    margin: 0;
  }
  .found {
    color: #1a7f37;
    font-weight: 600;
  }
  .steps {
    margin: 0;
    padding-left: 22px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  form input {
    flex: 1 1 auto;
    min-width: 0;
  }
  .box {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .error-box {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  }
  .card-head {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .where {
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .spread {
    justify-content: space-between;
  }
  .link {
    align-self: flex-start;
    height: auto;
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    cursor: pointer;
  }
  .bottom {
    margin-top: auto;
    padding-top: 8px;
  }
  .tree {
    display: flex;
    flex-direction: column;
    flex: 0 1 auto;
    min-height: 6em;
  }
  .tree :global(.entries) {
    flex: 0 1 auto;
    max-height: none;
    min-height: 0;
  }
</style>
