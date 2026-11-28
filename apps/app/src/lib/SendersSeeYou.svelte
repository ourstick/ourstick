<script lang="ts">
  // "Senders see you as: a Mac. Change…" (ADR 0015, rule 4): a name only if
  // the receiver chooses to give one; the app never asks for it.
  import { onMount } from "svelte";
  import { KINDS, onPhone, receiverName, setReceiverName } from "./api";

  let name = $state<string | null>(null);
  let editing = $state(false);
  let typed = $state("");
  let failure = $state<string | null>(null);

  let kind = $state("computer");
  // A phone says no kind: its reports carry none of a phone's ("other"),
  // so a sender's line would call it a computer (ADR 0029).
  const phone = onPhone();

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
    {#if phone}
      <p class="muted">Leave it empty to give no name. Your phone's own name is never sent.</p>
    {:else}
      <p class="muted">Leave it empty to show only a {kind}. Your computer's own name is never sent.</p>
    {/if}
  {:else}
    <p class="muted">
      {#if phone}
        {#if name}Senders see you as: <bdi>{name}</bdi>.{:else}Senders see no name for you.{/if}
      {:else}
        Senders see you as: {#if name}<bdi>{name}</bdi> ({kind}){:else}a {kind}{/if}.
      {/if}
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
  /* As every other link of the app's: accent, no button's height. */
  .link {
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
  p {
    margin: 0;
  }
  .muted {
    color: var(--muted);
  }
</style>
