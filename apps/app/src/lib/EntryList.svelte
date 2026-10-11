<script lang="ts">
  import { removeEntry, type Entry } from "./api";
  import { size } from "./format";

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

  /** The entry whose removal is being asked about. */
  let removing = $state<string | null>(null);

  function sizeOf(entry: Entry): string {
    return entry.size === null ? "reading…" : size(entry.size);
  }

  async function remove(name: string) {
    removing = null;
    try {
      await removeEntry(stickId, name);
    } catch (error) {
      onFailure(String(error));
    }
    onChanged();
  }
</script>

<ul class="entries">
  {#each entries as entry (entry.name)}
    <li
      oncontextmenu={(event) => {
        event.preventDefault();
        removing = entry.name;
      }}
    >
      {#if removing === entry.name}
        {#if entry.linked}
          <span>Stop sharing {entry.name}{entry.is_dir ? "/" : ""}? It stays where it is.</span>
        {:else}
          <span>Remove {entry.name}{entry.is_dir ? "/" : ""} from the stick?</span>
        {/if}
        <span class="inline-actions">
          <button onclick={() => (removing = null)}>Cancel</button>
          <button class="danger" onclick={() => remove(entry.name)}>
            {entry.linked ? "Stop sharing" : "Remove"}
          </button>
        </span>
      {:else}
        <span class="name" class:faded={entry.left_out}>
          {entry.name}{entry.is_dir ? "/" : ""}
          {#if entry.linked}<span class="tag" title="Shared from where it is, not copied">linked</span>{/if}
          {#if entry.missing}<span class="tag warn" title="Its disk is unplugged, or it was moved. People still get what they had.">not available</span>{/if}
          {#if entry.left_out}<span class="tag warn" title="Not shared: another item on the stick has this name.">hidden: same name</span>{/if}
        </span>
        <span class="muted">{sizeOf(entry)}</span>
      {/if}
    </li>
  {/each}
</ul>

<style>
  /* The list takes the room in the middle and scrolls; the rest keeps its
     place. */
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1 1 auto;
    /* About three rows, however much else the view shows. */
    min-height: 110px;
    overflow-y: auto;
  }
  .entries li {
    /* Keep their height: the list scrolls, its rows do not squash. */
    flex-shrink: 0;
    font-weight: 400;
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--button-radius);
  }
  .entries li > .muted {
    white-space: nowrap;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    font-size: 0.8em;
    padding: 0 6px;
    margin-left: 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
  }
  .tag.warn {
    border-color: var(--danger);
  }
  .faded {
    opacity: 0.6;
  }
  .inline-actions {
    display: flex;
    gap: 6px;
  }
  .muted {
    color: var(--muted);
  }
</style>
