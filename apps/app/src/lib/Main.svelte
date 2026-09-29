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
    type Activity,
    type ShareProblem,
    type Stick,
  } from "./api";
  import { files, rate, size } from "./format";

  let current = $state<Stick | null>(null);
  let activity = $state<Activity>({ readers: 0, bytes_per_second: 0 });
  let problem = $state<ShareProblem | null>(null);
  let dropping = $state(false);
  let copying = $state(false);
  let failure = $state<string | null>(null);
  let removing = $state<string | null>(null);
  let pillMenu = $state(false);

  async function pillAction(action: () => Promise<void>) {
    pillMenu = false;
    try {
      await action();
    } catch (error) {
      failure = String(error);
    }
    await refresh();
  }

  async function refresh() {
    try {
      current = await loadStick();
      activity = current.sharing;
    } catch (error) {
      failure = String(error);
    }
  }

  async function drop(paths: string[]) {
    if (paths.length === 0) return;
    copying = true;
    failure = null;
    try {
      await addPaths(paths);
    } catch (error) {
      failure = String(error);
    } finally {
      copying = false;
      await refresh();
    }
  }

  async function remove(name: string) {
    removing = null;
    try {
      await removeEntry(name);
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

  let over = $derived(current ? Math.max(0, current.used - current.size) : 0);
  let fraction = $derived(current && current.size > 0 ? Math.min(1, current.used / current.size) : 0);
  let count = $derived(current?.entries.length ?? 0);
</script>

<main class="card window">
  <header class="row spread">
    <h1><span class="drive" aria-hidden="true">▭</span> {current?.name ?? "OurStick"}{current ? ` (${size(current.size)})` : ""}</h1>
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

  <ul class="entries">
    {#each current?.entries ?? [] as entry (entry.name)}
      <li
        oncontextmenu={(event) => {
          event.preventDefault();
          removing = entry.name;
        }}
      >
        {#if removing === entry.name}
          <span>Remove {entry.name}{entry.is_dir ? "/" : ""} from the stick?</span>
          <span class="inline-actions">
            <button onclick={() => (removing = null)}>Cancel</button>
            <button class="danger" onclick={() => remove(entry.name)}>Remove</button>
          </span>
        {:else}
          <span class="name">{entry.name}{entry.is_dir ? "/" : ""}</span>
          <span class="muted">{size(entry.size)}</span>
        {/if}
      </li>
    {/each}
  </ul>

  <section class="capacity">
    <div class="row spread">
      <span>Capacity</span>
      <span>{size(current?.used ?? 0)} / {size(current?.size ?? 0)}</span>
    </div>
    <div class="bar"><div class="fill" class:full={over > 0} style="width: {fraction * 100}%"></div></div>
    {#if over > 0}
      <p class="error">Stick is full, remove {size(over)}.</p>
    {/if}
  </section>

  {#if activity.readers > 0}
    <p class="activity">
      ● {activity.readers === 1 ? "Someone downloading" : `${activity.readers} downloading`} · {rate(activity.bytes_per_second)}
    </p>
  {/if}

  {#if current?.sharing.state === "off"}
    <p class="row spread">
      <span class="muted">Not sharing: nobody can download.</span>
      <button onclick={() => startSharingAgain()}>Start sharing</button>
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
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}

  <button class="share" onclick={() => openShare().catch((error) => (failure = String(error)))}>
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
  .drop {
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
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 180px;
    overflow-y: auto;
  }
  .entries li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--button-radius);
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
  .share {
    width: 100%;
    min-height: calc(var(--button-height) + 8px);
  }
  p {
    margin: 0;
  }
</style>
