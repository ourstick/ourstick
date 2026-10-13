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

  let forgetting = $state(false);
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function forget() {
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
  {#if entry.state === "wait_ended"}
    <p class="error">{entry.message}</p>
    <p class="muted">Ask the sender for the link again, or forget this one.</p>
  {:else if entry.state === "stopped"}
    <p class="error">{entry.message}</p>
  {:else}
    <p>
      Waiting for the sender. It comes by itself, all of it, when the sender's computer is on and OurStick is open
      there.
    </p>
    {#if entry.message}<p class="muted">{entry.message}</p>{/if}
    <p class="muted">Keep OurStick open: minimize it rather than quit.</p>
  {/if}
  {#if day}<p class="muted">Waiting since {day}.</p>{/if}

  {#if forgetting}
    <section class="ask">
      <strong>Forget this link?</strong>
      <p>Nothing of it has come yet; OurStick stops waiting for it.</p>
      {#if busy}<p class="muted">Forgetting… please wait.</p>{/if}
      <div class="actions">
        <button onclick={() => (forgetting = false)} disabled={busy}>Cancel</button>
        <button class="danger" onclick={forget} disabled={busy}>Forget</button>
      </div>
    </section>
  {:else}
    <p><button onclick={() => (forgetting = true)}>Forget this stick…</button></p>
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
</section>

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
  .ask {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .ask .actions {
    margin-top: 6px;
  }
  p {
    margin: 0;
  }
</style>
