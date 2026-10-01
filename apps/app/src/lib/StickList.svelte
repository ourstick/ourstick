<script lang="ts">
  import { newStick, type StickEntry } from "./api";

  let {
    entries,
    selected,
    creating = $bindable(false),
    onSelect,
  }: {
    entries: StickEntry[];
    selected: string | null;
    /** "New stick…" is open (also from the tray). */
    creating: boolean;
    onSelect: (stickId: string) => void;
  } = $props();

  let name = $state("");
  let busy = $state(false);
  let failure = $state<string | null>(null);

  /** What a stick is doing, in words for everyone. */
  function stateWords(entry: StickEntry): string {
    switch (entry.sharing) {
      case "on":
        if (entry.readers === 1) return "Someone downloading";
        if (entry.readers > 1) return `${entry.readers} downloading`;
        return "Sharing";
      case "starting":
        return "Getting ready…";
      case "busy":
        return entry.doing ?? "Busy…";
      default:
        return "Not sharing";
    }
  }

  function cancel() {
    creating = false;
    name = "";
    failure = null;
  }

  async function create(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    failure = null;
    try {
      const made = await newStick(name);
      cancel();
      onSelect(made);
    } catch (error) {
      failure = String(error);
    } finally {
      busy = false;
    }
  }
</script>

<nav class="sticks" aria-label="Your sticks">
  <ul>
    {#each entries as entry (entry.id)}
      <li>
        <button class="stick" class:selected={entry.id === selected} onclick={() => onSelect(entry.id)}>
          <span class="name">
            <span class="dot" class:on={entry.sharing === "on"} aria-hidden="true">●</span>
            {entry.name}
          </span>
          <span class="state" class:active={entry.readers > 0}>{stateWords(entry)}</span>
        </button>
      </li>
    {/each}
  </ul>

  {#if creating}
    <form onsubmit={create}>
      <label>
        Name of the new stick
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value={name} maxlength="60" required disabled={busy} autofocus />
      </label>
      {#if busy}<p class="muted">Making it… please wait.</p>{/if}
      {#if failure}<p class="error">{failure}</p>{/if}
      <div class="row">
        <button type="button" onclick={cancel} disabled={busy}>Cancel</button>
        <button class="primary" type="submit" disabled={busy || !name.trim()}>Make it</button>
      </div>
    </form>
  {:else}
    <button class="new" onclick={() => (creating = true)}>+ New stick…</button>
  {/if}
</nav>

<style>
  .sticks {
    width: 190px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    padding-right: 12px;
    border-right: 1px solid var(--border);
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
  .stick.selected {
    border-color: var(--border);
    background: color-mix(in srgb, var(--accent) 12%, var(--card));
  }
  .name {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .dot {
    color: var(--muted);
    font-size: 0.8em;
  }
  .dot.on {
    color: #2a9d4b;
  }
  .state {
    font-size: 0.85em;
    color: var(--muted);
  }
  .state.active {
    color: #2a9d4b;
  }
  .new {
    margin-top: auto;
  }
  form,
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  form {
    margin-top: auto;
  }
  form p {
    margin: 0;
  }
  form .row {
    justify-content: flex-end;
  }
</style>
