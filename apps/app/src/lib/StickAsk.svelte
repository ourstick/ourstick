<script lang="ts" module>
  /** What is asked before something you send is changed. */
  export type Ask = "rename" | "new-link" | "remove";

  /** What became of it: renamed, a new id, removed; null if cancelled. */
  export type Outcome = { kind: "renamed" } | { kind: "new-link"; id: string } | { kind: "removed" } | null;
</script>

<script lang="ts">
  // The dialogs of something you send (gui-spec, "Its dialogs"); each says
  // "… please wait." while it works.
  import { newLink, removeStick, renameStick } from "./api";

  let {
    stickId,
    name,
    ask,
    folder,
    onDone,
  }: {
    stickId: string;
    name: string;
    ask: Ask;
    /** An older stick's own folder (or disk image), where its files stay. */
    folder: string | null;
    onDone: (outcome: Outcome) => void;
  } = $props();

  // The field starts with the name as it was when asked.
  // svelte-ignore state_referenced_locally
  let newName = $state(name);
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function run(work: () => Promise<Outcome>) {
    busy = true;
    failure = null;
    try {
      onDone(await work());
    } catch (error) {
      failure = String(error);
    } finally {
      busy = false;
    }
  }

  function rename(event: SubmitEvent) {
    event.preventDefault();
    run(async () => {
      await renameStick(stickId, newName);
      return { kind: "renamed" };
    });
  }
</script>

<div class="dialog-back">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="stick-ask-title">
    {#if ask === "rename"}
      <form onsubmit={rename}>
        <h2 id="stick-ask-title">Rename “<bdi>{name}</bdi>”</h2>
        <label>
          New name
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={newName} maxlength="60" required disabled={busy} autofocus />
        </label>
        <p class="muted">Others see the new name when their copy next updates. The link stays the same.</p>
        {#if busy}<p class="muted">Renaming… please wait.</p>{/if}
        {#if failure}<p class="error">{failure}</p>{/if}
        <div class="actions">
          <button type="button" onclick={() => onDone(null)} disabled={busy}>Cancel</button>
          <button class="primary" type="submit" disabled={busy || !newName.trim()}>Rename</button>
        </div>
      </form>
    {:else if ask === "new-link"}
      <h2 id="stick-ask-title">Make a new link for “<bdi>{name}</bdi>”?</h2>
      <p>
        The old link stops working, for everyone. Send the new one only to the people who should still get it.
      </p>
      <p class="muted">Use this if the link reached someone it shouldn't have.</p>
      {#if busy}<p class="muted">Making a new link… please wait.</p>{/if}
      {#if failure}<p class="error">{failure}</p>{/if}
      <div class="actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button class="primary" autofocus onclick={() => onDone(null)} disabled={busy}>Cancel</button>
        <button
          class="danger-outline"
          disabled={busy}
          onclick={() => run(async () => ({ kind: "new-link", id: await newLink(stickId) }))}
        >
          Make a new link
        </button>
      </div>
    {:else}
      <h2 id="stick-ask-title">Remove “<bdi>{name}</bdi>” from OurStick?</h2>
      <p>
        Its link stops working for good. Your files stay where they are on this computer, and those who got them keep
        their copy.
      </p>
      {#if folder}
        <p>Its files stay in <b>{folder}</b>. Delete them yourself when you no longer need them.</p>
      {/if}
      {#if busy}<p class="muted">Removing… please wait.</p>{/if}
      {#if failure}<p class="error">{failure}</p>{/if}
      <div class="actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button class="primary" autofocus onclick={() => onDone(null)} disabled={busy}>Cancel</button>
        <button
          class="danger"
          disabled={busy}
          onclick={() =>
            run(async () => {
              await removeStick(stickId);
              return { kind: "removed" };
            })}
        >
          Remove
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  form,
  label {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
</style>
