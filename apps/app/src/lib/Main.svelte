<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import {
    addPaths,
    eject,
    mount,
    openInFileManager,
    openShare,
    startSharingAgain,
    removeEntry,
    stick as loadStick,
    sticks,
    type Activity,
    type Entry,
    type ShareProblem,
    type Stick,
  } from "./api";
  import { files, rate, size } from "./format";
  import DoesNotFit from "./DoesNotFit.svelte";

  let current = $state<Stick | null>(null);
  /** The stick shown: the app's first, until the window lists them all. */
  let stickId = $state<string | null>(null);
  let activity = $state<Activity>({ readers: 0, bytes_per_second: 0, relay_only: null });
  let problem = $state<ShareProblem | null>(null);
  let dropping = $state(false);
  let copying = $state(false);
  let failure = $state<string | null>(null);
  let removing = $state<string | null>(null);
  /** A drop that does not fit, offered to be shared as it is. */
  let tooBig = $state<{ paths: string[]; needed: number; left: number } | null>(null);
  let pillMenu = $state(false);

  async function pillAction(action: (stickId: string) => Promise<void>) {
    pillMenu = false;
    if (!stickId) return;
    try {
      await action(stickId);
    } catch (error) {
      failure = String(error);
    }
    await refresh();
  }

  async function refresh() {
    try {
      // A new link gives the stick a new id: look it up each time.
      stickId = (await sticks())[0]?.id ?? null;
      if (!stickId) return;
      current = await loadStick(stickId);
      activity = current.sharing;
    } catch (error) {
      failure = String(error);
    }
  }

  async function drop(paths: string[]) {
    if (paths.length === 0) return;
    copying = true;
    failure = null;
    tooBig = null;
    try {
      if (!stickId) return;
      const outcome = await addPaths(stickId, paths);
      if (outcome.kind === "does_not_fit") {
        tooBig = { paths, needed: outcome.needed, left: outcome.left };
      }
    } catch (error) {
      failure = String(error);
    } finally {
      copying = false;
      await refresh();
    }
  }

  async function tooBigClosed(why: string | null) {
    tooBig = null;
    failure = why;
    await refresh();
  }

  function sizeOf(entry: Entry): string {
    return entry.size === null ? "reading…" : size(entry.size);
  }

  async function remove(name: string) {
    removing = null;
    try {
      if (stickId) await removeEntry(stickId, name);
    } catch (error) {
      failure = String(error);
    }
    await refresh();
  }

  onMount(() => {
    refresh();
    const stops = [
      listen("stick-changed", refresh),
      listen("sharing-changed", refresh),
      listen("mount-changed", refresh),
      listen("sticks-changed", refresh),
      listen<Activity>("activity", (event) => (activity = event.payload)),
      listen<ShareProblem>("share-problem", (event) => {
        problem = event.payload.kind === "cleared" ? null : event.payload;
      }),
      getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "over" || event.payload.type === "enter") {
          dropping = true;
        } else if (event.payload.type === "drop") {
          dropping = false;
          drop(event.payload.paths);
        } else {
          dropping = false;
        }
      }),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  });

  // Like a real stick: what is on it out of what it could hold.
  let fraction = $derived(
    current && current.used + current.left > 0 ? current.used / (current.used + current.left) : 0,
  );
  let count = $derived(current?.entries.length ?? 0);
</script>

<main class="card window">
  <header class="row spread">
    <h1><span class="drive" aria-hidden="true">▭</span> {current?.name ?? "OurStick"}</h1>
    {#if current}
      <div class="pill-box">
        <button class="pill" onclick={() => (pillMenu = !pillMenu)}>
          <span class:on={current.mounted}>{current.mounted ? "●" : "○"}</span>
          {current.mounted ? "Mounted" : "Ejected"} ▾
        </button>
        {#if pillMenu}
          <div class="menu" role="menu">
            <button role="menuitem" onclick={() => pillAction(openInFileManager)} disabled={!current.mounted}>
              Open in file manager
            </button>
            {#if current.mounted}
              <button role="menuitem" onclick={() => pillAction(eject)}>Eject (keeps sharing)</button>
            {:else}
              <button role="menuitem" onclick={() => pillAction(mount)}>Mount</button>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  </header>

  <section class="drop" class:dropping aria-label="Drop files or folders here">
    <strong>{copying ? "Copying…" : "Drop files or folders here"}</strong>
    <span class="muted">{files(count)} · {size(current?.used ?? 0)}</span>
  </section>

  {#if tooBig}
    <DoesNotFit stickId={stickId ?? ""} {...tooBig} onClose={tooBigClosed} />
  {/if}

  <ul class="entries">
    {#each current?.entries ?? [] as entry (entry.name)}
      <li
        oncontextmenu={(event) => {
          event.preventDefault();
          removing = entry.name;
        }}
      >
        {#if removing === entry.name}
          {#if entry.linked}
            <span>Stop sharing {entry.name}{entry.is_dir ? "/" : ""}? It stays where it is.</span>
          {:else}
            <span>Remove {entry.name}{entry.is_dir ? "/" : ""} from the stick?</span>
          {/if}
          <span class="inline-actions">
            <button onclick={() => (removing = null)}>Cancel</button>
            <button class="danger" onclick={() => remove(entry.name)}>
              {entry.linked ? "Stop sharing" : "Remove"}
            </button>
          </span>
        {:else}
          <span class="name" class:faded={entry.left_out}>
            {entry.name}{entry.is_dir ? "/" : ""}
            {#if entry.linked}<span class="tag" title="Shared from where it is, not copied">linked</span>{/if}
            {#if entry.missing}<span class="tag warn" title="Its disk is unplugged, or it was moved. People still get what they had.">not available</span>{/if}
            {#if entry.left_out}<span class="tag warn" title="Not shared: another item on the stick has this name.">hidden: same name</span>{/if}
          </span>
          <span class="muted">{sizeOf(entry)}</span>
        {/if}
      </li>
    {/each}
  </ul>

  <section class="capacity">
    <div class="row spread">
      <span>{size(current?.used ?? 0)} used · {size(current?.left ?? 0)} left</span>
    </div>
    <div class="bar"><div class="fill" class:full={current?.left === 0} style="width: {fraction * 100}%"></div></div>
  </section>

  {#if activity.readers > 0}
    <p class="activity">
      ● {activity.readers === 1 ? "Someone downloading" : `${activity.readers} downloading`} · {rate(activity.bytes_per_second)}
    </p>
  {/if}
  {#if activity.relay_only}
    <p class="error">{activity.relay_only}</p>
  {/if}

  {#if current?.sharing.state === "off"}
    <p class="row spread">
      <span class="muted">Not sharing: nobody can download.</span>
      <button onclick={() => stickId && startSharingAgain(stickId)}>Start sharing</button>
    </p>
  {:else if current?.sharing.state === "starting"}
    <p class="muted">Getting ready to share…</p>
  {:else if current?.sharing.state === "failed"}
    <p class="error">Not sharing: {current.sharing.message}</p>
  {/if}
  {#if problem?.kind === "ejected"}
    <p class="muted">The stick is ejected; people still get its last version.</p>
  {:else if problem?.kind === "failed"}
    <p class="error">{problem.message}</p>
  {:else if problem?.kind === "seen_slowly"}
    <p class="muted">Some folders cannot be watched (too many); their changes are seen within a minute.</p>
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}

  <button
    class="share"
    onclick={() => stickId && openShare(stickId).catch((error) => (failure = String(error)))}
  >
    ⤴ Share
  </button>
</main>

<style>
  .window {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  header h1 {
    margin: 0;
  }
  .drive {
    font-weight: 400;
  }
  .tag {
    font-size: 0.8em;
    padding: 0 6px;
    margin-left: 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
  }
  .tag.warn {
    border-color: var(--danger);
  }
  .faded {
    opacity: 0.6;
  }
  .drop {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    min-height: 120px;
    border: 2px dashed var(--border);
    border-radius: var(--radius);
  }
  .drop.dropping {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  /* The list takes the room in the middle and scrolls; the rest keeps its
     place. */
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }
  .entries li {
    /* Keep their height: the list scrolls, its rows do not squash. */
    flex-shrink: 0;
    font-weight: 400;
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--button-radius);
  }
  .entries li > .muted {
    white-space: nowrap;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .inline-actions {
    display: flex;
    gap: 6px;
  }
  .spread {
    justify-content: space-between;
  }
  .pill-box {
    position: relative;
  }
  .pill {
    border-radius: 999px;
    white-space: nowrap;
  }
  .pill .on {
    color: #2a9d4b;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 1;
    display: flex;
    flex-direction: column;
    min-width: 220px;
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: var(--button-radius);
    box-shadow: 0 4px 16px rgb(0 0 0 / 15%);
  }
  .menu button {
    border: none;
    border-radius: 0;
    text-align: left;
  }
  .menu button + button {
    border-top: 1px solid var(--border);
  }
  .bar {
    height: 8px;
    margin-top: 6px;
    border-radius: 4px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--text);
  }
  .fill.full {
    background: var(--danger);
  }
  .activity {
    margin: 0;
    color: #2a9d4b;
  }
  .muted {
    color: var(--muted);
  }
  /* Always at the bottom of the window. */
  .share {
    width: 100%;
    min-height: calc(var(--button-height) + 8px);
    margin-top: auto;
    flex-shrink: 0;
  }
  p {
    margin: 0;
  }
</style>
