<script lang="ts">
  import {
    addReceived,
    chooseFolder,
    previewInside,
    previewLink,
    previewNeeds,
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
      // Another link starts with everything ticked; another folder keeps the ticks.
      if (asked !== looked) leftOut = [];
      preview = seen;
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

  /** The link changed: what was shown is not of it any more. */
  function edited() {
    if (text !== looked) {
      preview = null;
      parent = null;
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

  async function getIt() {
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

  /** A preview's entry as the ticks list it: its last part shown. */
  function row(entry: PreviewEntry): ChoiceRow {
    return { ...entry, shown: entry.name.slice(entry.name.lastIndexOf("/") + 1) };
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

<section class="add">
  <h1>Add a sharing link</h1>
  <form onsubmit={look}>
    <label>
      Paste the sharing link someone sent you
      <!-- svelte-ignore a11y_autofocus -->
      <input
        bind:value={text}
        oninput={edited}
        autofocus
        disabled={looking || adding}
        spellcheck="false"
        autocomplete="off"
      />
    </label>
    <div class="row">
      <button type="button" onclick={onCancel} disabled={adding}>Cancel</button>
      <button class="primary" type="submit" disabled={looking || adding || !text.trim()}>Look at it</button>
    </div>
  </form>
  {#if looking}<p class="muted">Looking for the sender… please wait.</p>{/if}
  {#if failure}<p class="error">{failure}</p>{/if}

  {#if preview && !looking}
    <h2><bdi>{preview.name}</bdi></h2>
    <p class="row spread">
      <strong>What to get:</strong>
      <span class="row">
        <button onclick={() => (leftOut = [])} disabled={adding || leftOut.length === 0}>All</button>
        <button onclick={() => (leftOut = entries.map((entry) => entry.name))} disabled={adding || none}
          >None</button
        >
      </span>
    </p>
    <ChoiceTree
      rows={entries}
      bind:leftOut
      bind:none
      {loadInside}
      disabled={adding}
      onFailure={(why) => (failure = why)}
    />
    <p class="muted">New things the sender adds will come too.</p>
    <p class="row spread">
      <span class="muted path"><PathText label="Save to" path={preview.folder} /></span>
      <button onclick={choose} disabled={adding}>Choose…</button>
    </p>
    {#if preview.earlier}
      <p class="muted">You received this stick here before: it goes on in the same folder.</p>
    {:else if preview.earlier_elsewhere}
      <p class="muted">
        You received this stick before, in <bdi>{preview.earlier_elsewhere}</bdi>. The files there stay there; it starts again in the
        new folder.
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
    {#if adding}<p class="muted">Adding… please wait.</p>{/if}
    <div class="row end">
      <button class="primary" onclick={getIt} disabled={adding || !fits || none}>Get it</button>
    </div>
  {/if}
</section>

<style>
  .add {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
  }
  h1,
  h2 {
    margin: 0;
  }
  h2 {
    font-size: 1.1em;
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
