<script lang="ts">
  import {
    forgetReceived,
    openReceivedFolder,
    pauseReceived,
    resumeReceived,
    type ReceivedEntry,
  } from "./api";
  import { since, size } from "./format";
  import PathText from "./PathText.svelte";
  import LeftOutWords from "./LeftOutWords.svelte";
  import SavedAsNote from "./SavedAsNote.svelte";
  import ChooseWhat from "./ChooseWhat.svelte";

  let {
    entry,
    onForgotten,
  }: {
    entry: ReceivedEntry;
    onForgotten: () => void;
  } = $props();

  let menu = $state(false);
  let forgetting = $state(false);
  let choosing = $state(false);
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function act(action: () => Promise<void>) {
    menu = false;
    busy = true;
    failure = null;
    try {
      await action();
    } catch (error) {
      failure = String(error);
    } finally {
      busy = false;
    }
  }

  async function forget() {
    await act(() => forgetReceived(entry.id));
    if (!failure) onForgotten();
  }

  /** What it is doing, in a sentence. */
  let doing = $derived.by(() => {
    switch (entry.state) {
      case "starting":
        return "Getting ready…";
      case "downloading":
        return entry.total > 0 ? `Downloading: ${size(entry.done)} of ${size(entry.total)}` : "Downloading…";
      case "getting_more":
        // The sender is still reading its folder: all it has said so far is
        // here, and nothing is being fetched now.
        return "Up to date so far. Anything more the sender adds will come by itself.";
      case "up_to_date":
        return "Up to date. New files from the sender come by themselves while OurStick is open.";
      case "waiting":
        return `Waiting${entry.since_ms ? " " + since(entry.since_ms) : ""}. ${entry.message ?? ""}`;
      case "paused":
        return "Paused: nothing new comes until you resume it.";
      case "stopped":
        return entry.message ?? "Stopped.";
      default:
        return "";
    }
  });
  let fraction = $derived(entry.total > 0 ? entry.done / entry.total : 0);
</script>

<svelte:window onkeydown={(event) => event.key === "Escape" && (menu = false)} />

<section class="view">
  <header class="row spread">
    <h1 title={entry.name}><bdi>{entry.name}</bdi></h1>
    <div class="menu-box">
      <button class="more" aria-label="More for this stick" aria-expanded={menu} onclick={() => (menu = !menu)}>⋯</button>
      {#if menu}
        <div class="menu" role="menu">
          <button
            role="menuitem"
            onclick={() => {
              menu = false;
              choosing = true;
            }}>Choose what to get…</button
          >
          <button
            role="menuitem"
            class="danger-text"
            onclick={() => {
              menu = false;
              forgetting = true;
            }}>Forget this stick…</button
          >
        </div>
      {/if}
    </div>
  </header>

  {#if choosing}
    <ChooseWhat stickId={entry.id} onDone={() => (choosing = false)} />
  {/if}

  {#if forgetting}
    <section class="ask">
      <strong>Forget “<bdi>{entry.name}</bdi>”?</strong>
      <p>It is no longer kept up to date. Its files stay in <bdi>{entry.folder}</bdi>.</p>
      {#if busy}<p class="muted">Forgetting… please wait.</p>{/if}
      <div class="actions">
        <button onclick={() => (forgetting = false)} disabled={busy}>Cancel</button>
        <button class="danger" onclick={forget} disabled={busy}>Forget</button>
      </div>
    </section>
  {/if}

  <p class="folder row spread">
    <span class="muted path"><PathText label="Saved in" path={entry.folder} /></span>
    <button onclick={() => act(() => openReceivedFolder(entry.id))}>Open folder</button>
  </p>

  <p class:error={entry.state === "stopped"} class="doing">{doing}</p>
  {#if entry.state === "downloading"}
    <div class="bar"><div class="fill" style="width: {fraction * 100}%"></div></div>
  {/if}
  {#if entry.state === "stopped"}
    <p><button onclick={() => act(() => resumeReceived(entry.id))} disabled={busy}>Try again</button></p>
  {:else if entry.paused}
    <p><button class="primary" onclick={() => act(() => resumeReceived(entry.id))} disabled={busy}>Resume</button></p>
  {:else if entry.state}
    <p><button onclick={() => act(() => pauseReceived(entry.id))} disabled={busy}>Pause</button></p>
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
  {#if entry.left_out.length > 0}
    {@const getting = entry.left_out.filter((name) => !name.here)}
    {@const updating = entry.left_out.filter((name) => name.here)}
    {#if getting.length > 0}
      <p class="muted">Not getting: <LeftOutWords list={getting} />.</p>
    {/if}
    {#if updating.length > 0}
      <p class="muted">
        Not updating: <LeftOutWords list={updating} /> ({updating.length === 1 ? "it stays" : "they stay"} in the
        folder).
      </p>
    {/if}
  {/if}
  <SavedAsNote list={entry.saved_as} />
</section>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    flex: 1 1 auto;
  }
  header h1 {
    margin: 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spread {
    justify-content: space-between;
  }
  .more {
    padding: 0 10px;
    font-weight: 700;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
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
  .bar {
    height: 8px;
    border-radius: 4px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
  }
  p {
    margin: 0;
  }
</style>
