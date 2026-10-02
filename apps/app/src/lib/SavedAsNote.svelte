<script lang="ts">
  // What is (or will be) saved under another name than at the sender
  // (ADR 0010): the first lines, then all of them on request. Names are the
  // sender's text, escaped by the app and isolated here.
  import type { SavedAs } from "./api";
  import { files } from "./format";

  let { lines, coming = false }: { lines: SavedAs[]; coming?: boolean } = $props();
  const FIRST = 10;
  let all = $state(false);
  let count = $derived(lines.reduce((sum, line) => sum + line.files, 0));
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
    {#if lines.length > FIRST}
      <button onclick={() => (all = !all)}>{all ? "Show fewer" : `Show all ${lines.length}`}</button>
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
