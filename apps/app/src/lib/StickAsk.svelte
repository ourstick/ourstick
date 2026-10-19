<script lang="ts" module>
  /** What is asked about a stick before it is changed. */
  export type Ask = "rename" | "new-link" | "remove";

  /** What became of it: renamed, a new id, removed; null if cancelled. */
  export type Outcome = { kind: "renamed" } | { kind: "new-link"; id: string } | { kind: "removed" } | null;
</script>

<script lang="ts">
  import { newLink, removeStick, renameStick } from "./api";

  let {
    stickId,
    name,
    ask,
    onDone,
  }: { stickId: string; name: string; ask: Ask; onDone: (outcome: Outcome) => void } = $props();

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

<section class="ask" aria-live="polite">
  {#if ask === "rename"}
    <form onsubmit={rename}>
      <label>
        New name for “{name}”
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value={newName} maxlength="60" required disabled={busy} autofocus />
      </label>
      {#if busy}<p class="muted">Renaming… please wait.</p>{/if}
      {#if failure}<p class="error">{failure}</p>{/if}
      <div class="actions">
        <button type="button" onclick={() => onDone(null)} disabled={busy}>Cancel</button>
        <button class="primary" type="submit" disabled={busy || !newName.trim()}>Rename</button>
      </div>
    </form>
  {:else if ask === "new-link"}
    <strong>Change the sharing link of “{name}”?</strong>
    <p>The old sharing link stops working. Send the new one to the people who should still have it. The files stay.</p>
    {#if busy}<p class="muted">Changing the sharing link… please wait.</p>{/if}
    {#if failure}<p class="error">{failure}</p>{/if}
    <div class="actions">
      <button onclick={() => onDone(null)} disabled={busy}>Cancel</button>
      <button
        class="danger"
        disabled={busy}
        onclick={() => run(async () => ({ kind: "new-link", id: await newLink(stickId) }))}
      >
        Change it
      </button>
    </div>
  {:else}
    <strong>Remove “{name}”?</strong>
    <p>
      Its sharing link stops working. Your files stay on this computer: its own folder with what is in it, and the folders
      shared as they are. Delete them yourself when you no longer need them.
    </p>
    {#if busy}<p class="muted">Removing… please wait.</p>{/if}
    {#if failure}<p class="error">{failure}</p>{/if}
    <div class="actions">
      <button onclick={() => onDone(null)} disabled={busy}>Cancel</button>
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
</section>

<style>
  .ask {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .ask p {
    margin: 0;
  }
  .ask .actions {
    margin-top: 6px;
  }
  form,
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
</style>
