<script lang="ts">
  // Something you received (gui-spec): its name, Open folder, ⋯; its
  // status; what it leaves out.
  import {
    chooseFolder,
    forgetReceived,
    moveReceived,
    onPhone,
    openReceivedFolder,
    pauseReceived,
    resumeReceived,
    type ReceivedEntry,
  } from "./api";
  import { checkedAt, lastParts, since, size, thisDevice } from "./format";
  import LeftOutWords from "./LeftOutWords.svelte";
  import SavedAsNote from "./SavedAsNote.svelte";
  import ChooseWhat from "./ChooseWhat.svelte";
  import LocalNetworkNote from "./LocalNetworkNote.svelte";
  import SendersSeeYou from "./SendersSeeYou.svelte";

  let {
    entry,
    onForgotten,
  }: {
    entry: ReceivedEntry;
    onForgotten: () => void;
  } = $props();

  let menu = $state(false);
  let removing = $state(false);
  let choosing = $state(false);
  let seenAs = $state(false);
  let moving = $state(false);
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

  /** "Move to another folder…": the system's picker, then it moves. */
  async function move() {
    menu = false;
    const into = await chooseFolder().catch((error) => {
      failure = String(error);
      return null;
    });
    if (!into) return;
    moving = true;
    await act(() => moveReceived(entry.id, into));
    moving = false;
  }

  async function remove() {
    await act(() => forgetReceived(entry.id));
    if (!failure) onForgotten();
  }

  let folder = $derived(lastParts(entry.folder ?? ""));
  let here = $derived(entry.total > 0 ? `${size(entry.done)} of ${size(entry.total)} here` : "");
  let percent = $derived(entry.total > 0 ? Math.floor((entry.done / entry.total) * 100) : 0);
</script>

<svelte:window onkeydown={(event) => event.key === "Escape" && (menu = false)} />

<section class="view">
  <header class="row">
    <h1 title={entry.name}><bdi>{entry.name}</bdi></h1>
    <button onclick={() => act(() => openReceivedFolder(entry.id))}>Open folder</button>
    <div class="menu-box">
      <button class="more" aria-label="More" aria-expanded={menu} onclick={() => (menu = !menu)}>⋯</button>
      {#if menu}
        <div class="menu" role="menu">
          {#if entry.paused}
            <button role="menuitem" onclick={() => act(() => resumeReceived(entry.id))}>Resume</button>
          {:else}
            <button role="menuitem" onclick={() => act(() => pauseReceived(entry.id))}>
              <span>Pause</span><span class="hint">Then “Resume” here, when you want.</span>
            </button>
          {/if}
          <button role="menuitem" onclick={() => ((menu = false), (choosing = true))}>
            <span>Choose what to get…</span><span class="hint">Take some folders out, or add them back.</span>
          </button>
          <button role="menuitem" onclick={move}>Move to another folder…</button>
          <button role="menuitem" onclick={() => ((menu = false), (seenAs = true))}>How the sender sees you…</button>
          <button role="menuitem" class="danger-text" onclick={() => ((menu = false), (removing = true))}>
            <span>Remove…</span><span class="hint">What you have stays in its folder.</span>
          </button>
        </div>
      {/if}
    </div>
  </header>

  {#if choosing}
    <ChooseWhat stickId={entry.id} onDone={() => (choosing = false)} />
  {/if}

  <div class="status" class:stopped={entry.state === "stopped"}>
    {#if entry.state === "downloading"}
      <strong>↓ Downloading… {percent}%</strong>
      <div class="bar"><div class="fill" style="width: {percent}%"></div></div>
      <span class="muted">{size(entry.done)} of {size(entry.total)}</span>
      {#if onPhone()}
        <span>Keep OurStick open until it finishes; it goes on from where it stopped when you open it again.</span>
      {:else}
        <span>You can close this window. It goes on by itself, also after a restart.</span>
        <span class="muted">When the sender adds or changes files, you get them too.</span>
      {/if}
    {:else if entry.state === "up_to_date"}
      <strong>✓ All here</strong>
      <span class="muted">New files from the sender come by themselves.</span>
    {:else if entry.state === "checked"}
      <strong>✓ All here</strong>
      <span class="muted">{entry.since_ms ? checkedAt(entry.since_ms) : ""}</span>
      <button onclick={() => act(() => resumeReceived(entry.id))} disabled={busy}>Check again</button>
    {:else if entry.state === "queued"}
      <strong>… Next in line{here ? ` · ${here}` : ""}</strong>
      <span class="muted">It starts when the download before it ends.</span>
    {:else if entry.state === "getting_more"}
      <strong>✓ All here so far</strong>
      <span class="muted">The sender is still counting a big folder. The rest comes by itself.</span>
    {:else if entry.state === "waiting"}
      <strong>! Waiting{here ? ` · ${here}` : ""}</strong>
      <span>
        {entry.since_ms ? `${since(entry.since_ms).replace(/^s/, "S")}: ` : ""}{entry.message ?? ""} It goes on by itself.
      </span>
    {:else if entry.state === "paused"}
      <strong>⏸ Paused{here ? ` · ${here}` : ""}</strong>
      <button class="primary" onclick={() => act(() => resumeReceived(entry.id))} disabled={busy}>Resume</button>
    {:else if entry.state === "stopped"}
      <strong class="error">✕ Stopped</strong>
      <span>{entry.message ?? `Something on ${thisDevice()} failed. What came so far is safe.`}</span>
      <button onclick={() => act(() => resumeReceived(entry.id))} disabled={busy}>Try again</button>
    {:else}
      <strong>Getting ready…</strong>
    {/if}
  </div>
  {#if entry.local_network_declined}
    <LocalNetworkNote other="the sender" />
  {/if}
  {#if moving}<p class="muted">Moving… please wait. Big folders onto another disk take a while.</p>{/if}
  {#if failure}<p class="error">{failure}</p>{/if}

  {#if entry.left_out.length > 0}
    {@const getting = entry.left_out.filter((name) => !name.here)}
    {@const updating = entry.left_out.filter((name) => name.here)}
    <p class="muted">
      {#if getting.length > 0}Not getting: <LeftOutWords list={getting} />.{/if}
      {#if updating.length > 0}No longer updated: <LeftOutWords list={updating} /> ({updating.length === 1
          ? "it stays"
          : "they stay"} in your folder).{/if}
      <button class="link" onclick={() => (choosing = true)}>Change…</button>
    </p>
  {/if}
  <SavedAsNote list={entry.saved_as} />
</section>

{#if removing}
  <div class="dialog-back">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="remove-received-title">
      <h2 id="remove-received-title">Remove “<bdi>{entry.name}</bdi>”?</h2>
      <p>New files from the sender stop coming. What you have stays in <b><bdi>{folder}</bdi></b>.</p>
      <p class="muted">To get it again, open the link again.</p>
      {#if busy}<p class="muted">Removing… please wait.</p>{/if}
      <div class="actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button class="primary" autofocus onclick={() => (removing = false)} disabled={busy}>Cancel</button>
        <button class="danger" onclick={remove} disabled={busy}>Remove</button>
      </div>
    </div>
  </div>
{/if}

{#if seenAs}
  <div class="dialog-back">
    <div class="dialog" role="dialog" aria-modal="true" aria-label="How senders see you">
      <h2>How senders see you</h2>
      <SendersSeeYou />
      <div class="actions">
        <button onclick={() => (seenAs = false)}>Close</button>
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
  header {
    gap: 10px;
  }
  header h1 {
    flex: 1 1 auto;
    margin: 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .more {
    padding: 0 12px;
    font-weight: 700;
  }
  .menu button {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    height: auto;
    padding: 8px 12px;
  }
  .hint {
    font-size: 0.85em;
    color: var(--muted);
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
  .status.stopped {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  }
  .bar {
    align-self: stretch;
    height: 8px;
    border-radius: 4px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
  }
  .link {
    height: auto;
    min-height: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    cursor: pointer;
  }
  p {
    margin: 0;
  }
</style>
