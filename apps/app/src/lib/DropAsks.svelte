<script lang="ts">
  // What a drop onto a stick may ask (gui-spec, the drop zone): stop it while
  // it is looked at or copied, the drops waiting after it, "already on the
  // stick" and "does not fit".
  import DoesNotFit from "./DoesNotFit.svelte";
  import AlreadyThere from "./AlreadyThere.svelte";
  import { answerAlreadyThere, cancelQueued, dropsOf, queuedNames, stopDrop } from "./stickDrops.svelte";

  let {
    stickId,
    onTooBigClosed,
  }: {
    stickId: string;
    onTooBigClosed: (why: string | null) => void;
  } = $props();

  let drops = $derived(dropsOf(stickId));
</script>

{#if drops.copying}
  <p class="row spread">
    <span class="muted">
      {drops.stopping ? "Stopping…" : "A big folder? Share it as it is: from where it is, not copied."}
    </span>
    <span class="row">
      <button onclick={() => stopDrop(stickId, false)} disabled={drops.stopping}>Cancel</button>
      <button onclick={() => stopDrop(stickId, true)} disabled={drops.stopping}>Share it as it is</button>
    </span>
  </p>
{/if}

{#if drops.queue.length > 0}
  <p class="row spread">
    <span class="muted"
      >Next: {#each queuedNames(drops) as name, index (index)}{index > 0 ? ", " : ""}<bdi>{name}</bdi>{/each}</span
    >
    <button onclick={() => cancelQueued(stickId)}>Don't add these</button>
  </p>
{/if}

{#if drops.alreadyThere}
  <AlreadyThere names={drops.alreadyThere.names} onAnswer={(choice) => answerAlreadyThere(stickId, choice)} />
{/if}

{#if drops.tooBig}
  <DoesNotFit {stickId} {...drops.tooBig} onClose={onTooBigClosed} />
{/if}

<style>
  p {
    margin: 0;
    gap: 8px;
    flex-shrink: 0;
  }
  .spread {
    justify-content: space-between;
  }
  /* The words beside them wrap; a button's own words never do. */
  button {
    white-space: nowrap;
  }
</style>
