<script lang="ts">
  import type { ReceivedEntry } from "./api";
  import { receivedShort } from "./format";
  import SendersSeeYou from "./SendersSeeYou.svelte";

  let {
    entries,
    selected,
    adding,
    onSelect,
    onAdd,
  }: {
    entries: ReceivedEntry[];
    selected: string | null;
    adding: boolean;
    onSelect: (stickId: string) => void;
    onAdd: () => void;
  } = $props();
</script>

<section class="received" aria-label="Sticks you received">
  <h2>Received</h2>
  <ul>
    {#each entries as entry (entry.id)}
      <li>
        <button class="stick" class:selected={entry.id === selected} onclick={() => onSelect(entry.id)}>
          <!-- The sender's name, isolated: its direction never spills over. -->
          <bdi class="name">{entry.name}</bdi>
          <span class="state" class:stopped={entry.state === "stopped"}>
            {receivedShort(entry)}
          </span>
        </button>
      </li>
    {/each}
  </ul>
  <button class="add" class:selected={adding} onclick={onAdd}>+ Add a sharing link…</button>
  {#if entries.length > 0}<SendersSeeYou />{/if}
</section>

<style>
  .received {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
  }
  h2 {
    margin: 4px 0 0;
    font-size: 0.85em;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
    min-height: 0;
  }
  .stick {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 6px 8px;
    border-color: transparent;
    text-align: left;
  }
  .stick.selected,
  .add.selected {
    border-color: var(--border);
    background: color-mix(in srgb, var(--accent) 12%, var(--card));
  }
  .name {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
    unicode-bidi: isolate;
  }
  .state {
    font-size: 0.85em;
    color: var(--muted);
  }
  .state.stopped {
    color: var(--danger);
  }
</style>
