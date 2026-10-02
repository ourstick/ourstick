<script lang="ts">
  // "Choose what to get…" for a received stick (ADR 0012): its top-level
  // entries with their ticks, saved whole.
  import { onMount } from "svelte";
  import { chooseWhatToGet, receivedContents, type ChoiceEntry } from "./api";
  import { size } from "./format";

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
  let saving = $state(false);
  let failure = $state<string | null>(null);
  let unticked = $state<Set<string>>(new Set());

  onMount(async () => {
    try {
      const view = await receivedContents(stickId);
      entries = view.entries;
      known = view.known;
      unticked = new Set(view.entries.filter((entry) => !entry.ticked).map((entry) => entry.name));
    } catch (error) {
      failure = String(error);
    } finally {
      loading = false;
    }
  });

  function tick(name: string, on: boolean) {
    const next = new Set(unticked);
    if (on) next.delete(name);
    else next.add(name);
    unticked = next;
  }

  /** What is un-ticked now and already here: it stays, no longer updated. */
  let staying = $derived(entries.filter((entry) => unticked.has(entry.name) && entry.here));

  async function save() {
    saving = true;
    failure = null;
    try {
      await chooseWhatToGet(stickId, [...unticked]);
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
  {:else}
    {#if !known}
      <p class="muted">
        The rest of the list comes once the sender's computer is reachable. For now only what you chose not to get is
        listed.
      </p>
    {/if}
    {#if known && entries.length > 0}
      <p class="row">
        <button onclick={() => (unticked = new Set())} disabled={saving}>All</button>
        <button onclick={() => (unticked = new Set(entries.map((entry) => entry.name)))} disabled={saving}
          >None</button
        >
      </p>
    {/if}
    <ul class="entries">
      {#each entries as entry (entry.name)}
        <li>
          <label class="tick">
            <input
              type="checkbox"
              checked={!unticked.has(entry.name)}
              onchange={(event) => tick(entry.name, event.currentTarget.checked)}
              disabled={saving}
            />
            <bdi class="name">{entry.shown}{entry.is_dir ? "/" : ""}</bdi>
          </label>
          {#if known && !entry.on_stick}
            <span class="muted size">not on the stick now</span>
          {:else if entry.on_stick}
            <span class="muted size">{size(entry.bytes)}</span>
          {/if}
        </li>
      {/each}
    </ul>
    <p class="muted">New things the sender adds will come too.</p>
    {#if staying.length > 0}
      <p class="muted">
        {#each staying as entry, index (entry.name)}{index > 0 ? ", " : ""}<bdi>{entry.shown}</bdi>{/each}
        {staying.length === 1 ? "stays" : "stay"} in your folder, but
        {staying.length === 1 ? "is" : "are"} no longer updated. OurStick never deletes anything.
      </p>
    {/if}
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
  <div class="actions">
    <button onclick={onDone} disabled={saving}>Cancel</button>
    <button class="primary" onclick={save} disabled={saving || loading}>Save</button>
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
  }
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 16em;
    overflow-y: auto;
  }
  .entries li {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--button-radius);
  }
  .tick {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    unicode-bidi: isolate;
  }
  .size {
    flex-shrink: 0;
    white-space: nowrap;
  }
  p {
    margin: 0;
  }
</style>
