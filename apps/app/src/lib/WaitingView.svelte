<script lang="ts">
  // A link waited for (ADR 0025): pasted while its sender was away, it
  // comes by itself, everything ticked, once the sender is back.
  import { forgetReceived, type ReceivedEntry } from "./api";

  let {
    entry,
    onForgotten,
  }: {
    entry: ReceivedEntry;
    onForgotten: () => void;
  } = $props();

  let removing = $state(false);
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function remove() {
    busy = true;
    failure = null;
    try {
      await forgetReceived(entry.id);
      onForgotten();
    } catch (error) {
      failure = String(error);
    } finally {
      busy = false;
    }
  }

  let day = $derived(
    entry.since_ms ? new Date(entry.since_ms).toLocaleDateString([], { day: "numeric", month: "long" }) : null,
  );
</script>

<section class="view">
  <h1 title={entry.name}><bdi>{entry.name}</bdi></h1>
  <div class="status">
    {#if entry.state === "wait_ended"}
      <strong class="error">The sender did not answer for 30 days</strong>
      <span>Ask them to open OurStick, or for a new link.</span>
    {:else if entry.state === "stopped"}
      <strong class="error">✕ Stopped</strong>
      <span>{entry.message}</span>
    {:else}
      <strong>… Waiting for the sender{day ? ` since ${day}` : ""}</strong>
      <span>Its name and files appear once the sender is back. Leave OurStick open: minimize it, don't quit.</span>
    {/if}
    <button class:danger-outline={entry.state === "wait_ended"} onclick={() => (removing = true)}>Remove…</button>
  </div>
  {#if failure}<p class="error">{failure}</p>{/if}
</section>

{#if removing}
  <div class="dialog-back">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="remove-waiting-title">
      <h2 id="remove-waiting-title">Stop waiting for this link?</h2>
      <p>Nothing of it has come yet. To get it later, open the link again.</p>
      {#if busy}<p class="muted">Removing… please wait.</p>{/if}
      <div class="actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button class="primary" autofocus onclick={() => (removing = false)} disabled={busy}>Cancel</button>
        <button class="danger" onclick={remove} disabled={busy}>Remove</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    flex: 1 1 auto;
  }
  h1 {
    margin: 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  p {
    margin: 0;
  }
</style>
