<script lang="ts">
  // "Choose what to get…" for a received stick (ADR 0012 and its Amendment
  // 1): its entries as a tree of ticks, saved whole.
  import { onMount } from "svelte";
  import { chooseWhatToGet, receivedContents, type ChoiceEntry } from "./api";
  import ChoiceTree, { type ChoiceRow } from "./ChoiceTree.svelte";

  let {
    stickId,
    onDone,
  }: {
    stickId: string;
    onDone: () => void;
  } = $props();

  let entries = $state<ChoiceEntry[]>([]);
  let known = $state(true);
  let loading = $state(true);
  /** The list could not be read: saving would send a choice never shown. */
  let unread = $state(false);
  let saving = $state(false);
  let failure = $state<string | null>(null);
  /** The paths left out, exactly as the stick names them. */
  let leftOut = $state<string[]>([]);
  let none = $state(false);
  let shownRows = $state<ChoiceRow[]>([]);
  /** Paths left out when the window opened whose files are here. */
  let stayingBefore = $state<{ name: string; shown: string }[]>([]);

  onMount(async () => {
    try {
      const view = await receivedContents(stickId);
      entries = view.entries;
      known = view.known;
      // Every path left out, also inside folders not opened: a save sends
      // the whole set.
      leftOut = view.left_out;
      stayingBefore = view.staying;
    } catch (error) {
      failure = String(error);
      unread = true;
    } finally {
      loading = false;
    }
  });

  async function loadInside(folder: string): Promise<ChoiceRow[]> {
    return (await receivedContents(stickId, folder)).entries;
  }

  /** What is left out now and already here: it stays, no longer updated
   * (inside a folder not opened too). */
  let staying = $derived.by(() => {
    const here = new Map<string, string>();
    for (const row of shownRows) if (row.here && leftOut.includes(row.name)) here.set(row.name, row.shown);
    for (const path of stayingBefore) if (leftOut.includes(path.name)) here.set(path.name, path.shown);
    return [...here].map(([name, shown]) => ({ name, shown }));
  });

  async function save() {
    saving = true;
    failure = null;
    try {
      await chooseWhatToGet(stickId, leftOut);
      onDone();
    } catch (error) {
      failure = String(error);
      saving = false;
    }
  }
</script>

<section class="ask">
  <strong>What to get:</strong>
  {#if loading}
    <p class="muted">Looking… please wait.</p>
  {:else if !unread}
    {#if !known}
      <p class="muted">
        The rest of the list comes once the sender's computer is on. For now only what you chose not to get is
        listed.
      </p>
    {/if}
    {#if known && entries.length > 0}
      <p class="row">
        <button onclick={() => (leftOut = [])} disabled={saving}>All</button>
        <button onclick={() => (leftOut = entries.map((entry) => entry.name))} disabled={saving}>None</button>
      </p>
    {/if}
    <ChoiceTree
      rows={entries}
      bind:leftOut
      bind:none
      bind:shownRows
      loadInside={known ? loadInside : null}
      disabled={saving}
      {known}
      onFailure={(why) => (failure = why)}
    />
    <p class="muted">New things the sender adds will come too.</p>
    {#if staying.length > 0}
      <p class="muted">
        {#each staying as entry, index (entry.name)}{index > 0 ? ", " : ""}<bdi>{entry.shown}</bdi>{/each}
        {staying.length === 1 ? "stays" : "stay"} in your folder, but
        {staying.length === 1 ? "is" : "are"} no longer updated. OurStick never deletes anything.
      </p>
    {/if}
    {#if none}<p class="error">Tick at least one thing.</p>{/if}
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
  <div class="actions">
    <button onclick={onDone} disabled={saving}>Cancel</button>
    <button class="primary" onclick={save} disabled={saving || loading || unread || none}>Save</button>
  </div>
</section>

<style>
  .ask {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    /* No height of its own: the list stops at 40vh by itself, and a cap
       here pushed Cancel and Save out of the box, under the status below,
       where they could not be clicked. The view scrolls instead. */
  }
  p {
    margin: 0;
  }
</style>
