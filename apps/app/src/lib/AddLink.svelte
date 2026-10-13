<script lang="ts">
  import { onMount, tick } from "svelte";
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
  import { size } from "./format";
  import ChoiceTree, { type ChoiceRow } from "./ChoiceTree.svelte";
  import PathText from "./PathText.svelte";
  import SavedAsNote from "./SavedAsNote.svelte";

  let {
    onAdded,
    onCancel,
  }: {
    onAdded: (stickId: string) => void;
    onCancel: () => void;
  } = $props();

  let text = $state("");
  /** The text the preview is of: "Get it" adds exactly that link. */
  let looked = "";
  let parent = $state<string | null>(null);
  let preview = $state<PreviewView | null>(null);
  /** The paths left out (ADR 0012 and its Amendment 1), exactly as the
   * stick names them. */
  let leftOut = $state<string[]>([]);
  let none = $state(false);
  /** What would be written with this choice, as the core counts it. */
  let needs = $state(0);
  /** The link whose sender was away (ADR 0025): it can be waited for. */
  let away = $state<string | null>(null);
  let looking = $state(false);
  let adding = $state(false);
  let failure = $state<string | null>(null);

  async function look(event?: SubmitEvent) {
    event?.preventDefault();
    looking = true;
    failure = null;
    const asked = text;
    try {
      const seen = await previewLink(asked, parent);
      if (seen.kind === "sender_away") {
        // Another folder of a link looked at: said, the preview kept.
        if (preview && asked === looked) {
          failure = "The sender's computer is off, or OurStick is not open there. Try again when it is.";
        } else {
          preview = null;
          away = asked;
        }
        return;
      }
      away = null;
      // Another link starts with everything ticked; another folder keeps the ticks.
      if (asked !== looked) leftOut = [];
      preview = seen.preview;
      looked = asked;
    } catch (error) {
      // A new link that fails shows nothing of the old one; a new folder
      // that fails keeps the preview as it was.
      if (asked !== looked) preview = null;
      failure = String(error);
    } finally {
      looking = false;
    }
  }

  let field = $state<HTMLTextAreaElement>();

  /** As tall as the link: all of it shows, however long. */
  function grow() {
    if (!field) return;
    field.style.height = "auto";
    field.style.height = `${field.scrollHeight}px`;
  }

  /** The link changed: what was shown is not of it any more. */
  function edited() {
    grow();
    if (text !== looked) {
      preview = null;
      parent = null;
    }
  }

  // A link the user just copied (from a message) is put in for them.
  onMount(async () => {
    const copied = await linkOnClipboard().catch(() => null);
    if (copied && !text) {
      text = copied;
      await tick();
      grow();
      field?.select();
    }
  });

  /** Enter looks at the link, as in a one-line field; the link has no lines. */
  function keydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      field?.form?.requestSubmit();
    }
  }

  async function choose() {
    const picked = await chooseFolder().catch((error) => {
      failure = String(error);
      return null;
    });
    if (picked) {
      parent = picked;
      await look();
    }
  }

  /** "Wait for it": kept, and received by itself once the sender is back. */
  async function waitForIt() {
    if (away === null || adding || looking) return;
    adding = true;
    failure = null;
    try {
      onAdded(await waitForLink(away));
    } catch (error) {
      failure = String(error);
    } finally {
      adding = false;
    }
  }

  async function getIt() {
    if (!preview || adding || looking) return;
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

  /** A preview's entry as the ticks list it: its last part shown. */
  function row(entry: PreviewEntry): ChoiceRow {
    return entry;
  }

  /** Largest first: size is why the list is opened. */
  let entries = $derived(preview ? [...preview.entries].sort((a, b) => b.bytes - a.bytes).map(row) : []);

  async function loadInside(folder: string): Promise<ChoiceRow[]> {
    return (await previewInside(folder)).sort((a, b) => b.bytes - a.bytes).map(row);
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
  // While the sender still counts, what is known is the least it needs:
  // more than is left already does not fit.
  let fits = $derived(preview !== null && needs <= preview.left);
</script>

<!-- The link and its buttons until it is looked at; then only what it
     brings, with "Get it" always in sight below the list. -->
<section class="add">
  {#if !preview}
    <h1>Add a sharing link</h1>
    <form onsubmit={look}>
      <label>
        Paste the sharing link someone sent you
        <!-- svelte-ignore a11y_autofocus -->
        <textarea
          bind:this={field}
          bind:value={text}
          oninput={edited}
          onkeydown={keydown}
          rows="1"
          autofocus
          disabled={looking || adding}
          spellcheck="false"
          autocomplete="off"
        ></textarea>
      </label>
      <div class="row">
        <button type="button" onclick={onCancel} disabled={adding}>Cancel</button>
        {#if away !== null && away === text}
          <button type="submit" disabled={looking || adding}>Look again</button>
          <button class="primary" type="button" onclick={waitForIt} disabled={looking || adding}>Wait for it</button>
        {:else}
          <button class="primary" type="submit" disabled={looking || adding || !text.trim()}>Look at it</button>
        {/if}
      </div>
    </form>
    {#if away !== null && away === text && !looking}
      <p class="error">The sender's computer is off, or OurStick is not open there.</p>
      <p class="muted">
        Wait for it: it comes by itself, all of it, once the sender's computer is on and OurStick is open there.
      </p>
    {/if}
  {/if}
  {#if looking}<p class="muted">Looking for the sender… please wait.</p>{/if}
  {#if failure && !preview}<p class="error">{failure}</p>{/if}

  <!-- Kept while another folder is looked at, everything off: what was
       opened stays open. -->
  {#if preview}
    <h1><bdi>{preview.name}</bdi></h1>
    <p class="row spread">
      <strong>What to get:</strong>
      <span class="row">
        <button onclick={() => (leftOut = [])} disabled={adding || looking || leftOut.length === 0}>All</button>
        <button onclick={() => (leftOut = entries.map((entry) => entry.name))} disabled={adding || looking || none}
          >None</button
        >
      </span>
    </p>
    <div class="tree">
      <ChoiceTree
        rows={entries}
        bind:leftOut
        bind:none
        {loadInside}
        disabled={adding || looking}
        onFailure={(why) => (failure = why)}
      />
    </div>
    <div class="foot">
      <p class="muted">New things the sender adds will come too.</p>
      <p class="row spread">
        <span class="muted path"><PathText label="Save to" path={preview.folder} /></span>
        <button onclick={choose} disabled={adding || looking}>Choose…</button>
      </p>
      {#if preview.earlier}
        <p class="muted">You received this stick here before: it goes on in the same folder.</p>
      {:else if preview.earlier_elsewhere}
        <p class="muted">
          You received this stick before, in <bdi>{preview.earlier_elsewhere}</bdi>. The files there stay there; it starts again in
          the new folder.
        </p>
      {/if}
      <SavedAsNote list={preview.saved_as} coming />
      {#if none}
        <p class="error">Tick at least one thing.</p>
      {:else if preview.partial && fits}
        <p class="muted">The sender is still counting its files: more is coming.</p>
      {:else if preview.partial}
        <p class="error">
          Needs at least {size(needs)} so far, and only {size(preview.left)} are left where it would be saved. Un-tick
          something, choose another place, or free some room.
        </p>
      {:else if fits}
        <p class="muted">Needs up to {size(needs)} · fits</p>
      {:else}
        <p class="error">
          Needs up to {size(needs)}, and only {size(preview.left)} are left where it would be saved. Un-tick something,
          choose another place, or free some room.
        </p>
      {/if}
      {#if failure}<p class="error">{failure}</p>{/if}
      {#if adding}<p class="muted">Adding… please wait.</p>{/if}
      <div class="row">
        <button onclick={onCancel} disabled={adding}>Cancel</button>
        <button class="primary" onclick={getIt} disabled={adding || looking || !fits || none}>Get it</button>
      </div>
    </div>
  {/if}
</section>

<style>
  /* The whole link shows: it wraps anywhere and the field grows with it. */
  textarea {
    resize: none;
    overflow: hidden;
    word-break: break-all;
  }
  .add {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
    overflow-y: auto;
  }
  /* The list is as tall as its rows, up to the room left, and then scrolls
     by itself: what is below it stays in sight. */
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
  .foot {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex-shrink: 0;
  }
  h1 {
    margin: 0;
  }
  form,
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .row {
    justify-content: flex-end;
  }
  .spread {
    justify-content: space-between;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    unicode-bidi: isolate;
  }
  p {
    margin: 0;
  }
</style>
