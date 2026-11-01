<script lang="ts">
  // "What is in it" (gui-spec, Something you send): each entry with its
  // size and where it is (a click shows it in the file manager), and its
  // `×`, asked about first. Nothing of the user's is ever deleted: an older
  // stick's own entries have no `×`; they are changed in the file manager.
  // The one folder a stick was made from has none either: without it the
  // stick would be nothing, so it is stopped as a whole. One not found
  // has "Find it…" (ADR 0029): chosen again where it is now, it keeps its
  // name and the link.
  import { findEntry, onPhone, removeEntry, type Entry } from "./api";
  import { showEntry } from "./sendApi";
  import { size, thisDevice } from "./format";

  let {
    stickId,
    entries,
    onChanged,
    onFailure,
  }: {
    stickId: string;
    entries: Entry[];
    onChanged: () => void;
    onFailure: (why: string) => void;
  } = $props();

  /** The entry whose `×` is being asked about. */
  let stopping = $state<Entry | null>(null);
  let busy = $state(false);

  /** One of an older stick's own entries with a linked entry's name is not sent. */
  let shown = $derived(entries.filter((entry) => !entry.left_out));
  /** A phone has no file manager to show an entry in (ADR 0029). */
  const phone = onPhone();

  function where(entry: Entry): string {
    const counted = entry.size === null ? "counting…" : size(entry.size);
    return entry.place ? `${counted} · ${entry.place}` : counted;
  }

  async function show(entry: Entry) {
    try {
      await showEntry(stickId, entry.name);
    } catch (error) {
      onFailure(String(error));
    }
  }

  /** "Find it…": chosen again where it is now, keeping its name and the link. */
  async function find(entry: Entry) {
    busy = true;
    try {
      if (await findEntry(stickId, entry.name)) onChanged();
    } catch (error) {
      onFailure(String(error));
    } finally {
      busy = false;
    }
  }

  async function stop(entry: Entry) {
    busy = true;
    try {
      await removeEntry(stickId, entry.name);
    } catch (error) {
      onFailure(String(error));
    } finally {
      busy = false;
      stopping = null;
    }
    onChanged();
  }
</script>

<ul class="entries">
  {#each shown as entry (`${entry.linked}/${entry.name}`)}
    <li class:missing={entry.missing}>
      <span class="icon" aria-hidden="true">{entry.missing ? "!" : entry.is_dir ? "📁" : "📄"}</span>
      <span class="text">
        <bdi class="name">{entry.name}</bdi>
        {#if entry.missing}
          <span class="warning">Not found. Was it moved, or is its disk unplugged? Those who have it keep it.</span>
        {:else if entry.linked && !phone}
          <button class="where" title="Show it in the file manager" onclick={() => show(entry)}>{where(entry)}</button>
        {:else}
          <span class="muted">{where(entry)}</span>
        {/if}
        {#if entry.at_top}
          <span class="muted">This is what you send. To stop, use ⋯ › Stop sending.</span>
        {/if}
      </span>
      {#if entry.linked && entry.missing}
        <span class="actions">
          <button disabled={busy} onclick={() => find(entry)}>Find it…</button>
          {#if !entry.at_top}
            <button disabled={busy} onclick={() => (stopping = entry)}>Remove it</button>
          {/if}
        </span>
      {:else if entry.linked && !entry.at_top}
        <button class="remove" aria-label="Stop sending {entry.name}" onclick={() => (stopping = entry)}>×</button>
      {/if}
    </li>
  {/each}
</ul>

{#if stopping}
  <div class="dialog-back">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="stop-entry-title">
      <h2 id="stop-entry-title">Stop sending “<bdi>{stopping.name}</bdi>”?</h2>
      <p>Those who have it keep it. It stays where it is on {thisDevice()}.</p>
      <div class="actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button class="primary" autofocus disabled={busy} onclick={() => (stopping = null)}>Cancel</button>
        <button class="danger" disabled={busy} onclick={() => stopping && stop(stopping)}>Stop sending</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  li.missing {
    border-color: color-mix(in srgb, var(--warning) 50%, var(--border));
  }
  .text {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name {
    overflow-wrap: anywhere;
  }
  .where {
    align-self: flex-start;
    height: auto;
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--muted);
    cursor: pointer;
    text-align: left;
  }
  .where:hover {
    text-decoration: underline;
  }
  .remove {
    padding: 0 10px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
  }
</style>
