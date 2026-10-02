<script lang="ts">
  // What is (or will be) saved under another name than at the sender
  // (ADR 0010): the first lines, then all of them on request. Names are the
  // sender's text, escaped by the app and isolated here.
  import type { SavedAsList } from "./api";
  import { files } from "./format";

  let { list, coming = false }: { list: SavedAsList; coming?: boolean } = $props();
  const FIRST = 10;
  let all = $state(false);
  let lines = $derived(list.lines);
  let count = $derived(list.files);
  let shown = $derived(all ? lines : lines.slice(0, FIRST));
</script>

{#if lines.length > 0}
  <section class="note">
    <p>
      {#if count === 1}
        1 file {coming ? "will have" : "has"} a slightly different name here than at the sender:
      {:else}
        {count} files {coming ? "will have" : "have"} slightly different names here than at the sender:
      {/if}
    </p>
    <ul>
      {#each shown as line (line.path + "\0" + line.saved_as)}
        <li>
          <bdi>{line.path}</bdi> is saved as <bdi>{line.saved_as}</bdi>{#if line.files > 1}<span class="muted">
              ({files(line.files)})</span
            >{/if}
        </li>
      {/each}
    </ul>
    {#if all && list.more > 0}
      <p class="muted">… and {list.more} more {list.more === 1 ? "name" : "names"} (see the folder).</p>
    {/if}
    {#if lines.length > FIRST}
      <button onclick={() => (all = !all)}>{all ? "Show fewer" : `Show all ${lines.length + list.more}`}</button>
    {/if}
  </section>
{/if}

<style>
  .note {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .note p {
    margin: 0;
  }
  ul {
    margin: 0;
    padding-left: 18px;
    max-height: 12em;
    overflow-y: auto;
  }
  li {
    overflow-wrap: anywhere;
  }
  button {
    align-self: flex-start;
  }
</style>
