<script lang="ts">
  // "Senders see you as: a Mac. Change…" (ADR 0015, rule 4): a name only if
  // the receiver chooses to give one; the app never asks for it.
  import { onMount } from "svelte";
  import { KINDS, receiverName, setReceiverName } from "./api";

  let name = $state<string | null>(null);
  let editing = $state(false);
  let typed = $state("");
  let failure = $state<string | null>(null);

  let kind = $state("computer");

  onMount(async () => {
    const seen = await receiverName().catch(() => null);
    if (seen) {
      kind = KINDS[seen.kind];
      name = seen.name;
    }
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    try {
      await setReceiverName(typed);
      name = typed.trim() || null;
      editing = false;
      failure = null;
    } catch (error) {
      failure = String(error);
    }
  }
</script>

<div class="seen">
  {#if editing}
    <form onsubmit={save}>
      <label class="muted">
        Your name for senders (optional)
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value={typed} maxlength="40" autofocus spellcheck="false" autocomplete="off" />
      </label>
      <span class="row">
        <button type="button" onclick={() => (editing = false)}>Cancel</button>
        <button class="primary" type="submit">Save</button>
      </span>
    </form>
    <p class="muted">Leave it empty to show only a {kind}. Your computer's own name is never sent.</p>
  {:else}
    <p class="muted">
      Senders see you as: {#if name}<bdi>{name}</bdi> ({kind}){:else}a {kind}{/if}.
      <button
        class="link"
        onclick={() => {
          typed = name ?? "";
          editing = true;
        }}>Change…</button
      >
    </p>
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
</div>

<style>
  .seen {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 0.9em;
  }
  form,
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .link {
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    text-decoration: underline;
  }
  p {
    margin: 0;
  }
  .muted {
    color: var(--muted);
  }
</style>
