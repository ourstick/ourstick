<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import {
    chooseForStick,
    eject,
    mount,
    openDialog,
    openInFileManager,
    openShare,
    startSharingAgain,
    stick as loadStick,
    type About,
    type Activity,
    type ShareProblem,
    type Stick,
  } from "./api";
  import { files, rate, size } from "./format";
  import DoesNotFit from "./DoesNotFit.svelte";
  import AlreadyThere from "./AlreadyThere.svelte";
  import EntryList from "./EntryList.svelte";
  import ReaderList from "./ReaderList.svelte";
  import LocalNetworkNote from "./LocalNetworkNote.svelte";
  import StickMenu, { type Pick } from "./StickMenu.svelte";
  import StickAsk, { type Ask, type Outcome } from "./StickAsk.svelte";
  import { answerAlreadyThere, cancelQueued, closeTooBig, dropOn, dropsOf, queuedNames } from "./stickDrops.svelte";

  let {
    stickId,
    onOutcome,
  }: {
    stickId: string;
    /** A rename, new link or removal done here: the list follows. */
    onOutcome: (outcome: Outcome) => void;
  } = $props();

  let current = $state<Stick | null>(null);
  let activity = $state<Activity>({
    readers: 0,
    bytes_per_second: 0,
    preparing: false,
    relay_only: null,
    giving_way: false,
    local_network_declined: false,
  });
  let dropping = $state(false);
  /** The stick's drops: kept per stick, so they survive this view. */
  let drops = $derived(dropsOf(stickId));
  let failure = $state<string | null>(null);
  /** The one menu open, if any: the drive pill's or the stick's "⋯". */
  let openMenu = $state<"pill" | "more" | "add" | null>(null);
  let asking = $state<Ask | null>(null);

  async function act(action: (stickId: string) => Promise<void>) {
    openMenu = null;
    try {
      await action(stickId);
    } catch (error) {
      failure = String(error);
    }
    await refresh();
  }

  /** Answers to an older look are dropped: they arrive in any order. */
  let looks = 0;

  async function refresh() {
    const look = ++looks;
    try {
      const loaded = await loadStick(stickId);
      if (look !== looks) return;
      current = loaded;
      activity = loaded.sharing;
    } catch (error) {
      if (look === looks) failure = String(error);
    }
  }

  /** A click outside a menu, or Escape, closes it. */
  function closeMenus(event: MouseEvent | KeyboardEvent) {
    if (event instanceof KeyboardEvent) {
      if (event.key === "Escape") openMenu = null;
    } else if (!(event.target instanceof Element && event.target.closest(".menu-box"))) {
      openMenu = null;
    }
  }

  function picked(pick: Pick) {
    openMenu = null;
    if (pick === "stop") act((id) => openDialog(false, id));
    else if (pick === "start") act(startSharingAgain);
    else asking = pick;
  }

  function asked(outcome: Outcome) {
    asking = null;
    onOutcome(outcome);
    if (outcome?.kind === "renamed") refresh();
  }

  /** "Click to choose": what the system's picker gives is added as a drop. */
  async function choose(pick: "files" | "folders") {
    openMenu = null;
    try {
      await dropOn(stickId, await chooseForStick(pick));
    } catch (error) {
      failure = String(error);
    }
  }

  /** "3 folders · 1 file": what is on the stick's top level. */
  function contents(entries: { is_dir: boolean }[]): string {
    const folders = entries.filter((entry) => entry.is_dir).length;
    const plain = entries.length - folders;
    const parts = [];
    if (folders > 0) parts.push(folders === 1 ? "1 folder" : `${folders} folders`);
    if (plain > 0 || folders === 0) parts.push(files(plain));
    return parts.join(" · ");
  }

  async function tooBigClosed(why: string | null) {
    closeTooBig(stickId, why);
    await refresh();
  }

  /** OurStick gives way to another program reading the same disk. */
  let givingWay = $state(false);

  // A view per stick (the list makes it anew for another one).
  onMount(() => {
    refresh();
    const stops = [
      listen("stick-changed", refresh),
      listen("sharing-changed", refresh),
      listen("mount-changed", refresh),
      // Only this stick's.
      listen<About<Activity>>("activity", (event) => {
        if (event.payload.stick_id === stickId) activity = event.payload;
      }),
      // Told at each change only (ADR 0013).
      listen<About<{ giving_way: boolean }>>("giving-way", (event) => {
        if (event.payload.stick_id === stickId) givingWay = event.payload.giving_way;
      }),
      listen("problems-changed", refresh),
      getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "over" || event.payload.type === "enter") {
          dropping = true;
        } else if (event.payload.type === "drop") {
          dropping = false;
          void dropOn(stickId, event.payload.paths);
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
  let sharing = $derived(current?.sharing.state === "on" || current?.sharing.state === "starting");
  let problem = $derived<ShareProblem | null>(current?.problem ?? null);
</script>

<svelte:window onclick={closeMenus} onkeydown={closeMenus} />

<section class="view">
  <header class="row spread">
    <h1 title={current?.name}><span class="drive" aria-hidden="true">▭</span> {current?.name ?? ""}</h1>
    {#if current}
      <div class="row">
        <div class="menu-box">
          <button class="pill" aria-expanded={openMenu === "pill"} onclick={() => (openMenu = openMenu === "pill" ? null : "pill")}>
            <span class:on={current.mounted}>{current.mounted ? "●" : "○"}</span>
            {current.mounted ? "Mounted" : "Ejected"} ▾
          </button>
          {#if openMenu === "pill"}
            <div class="menu" role="menu">
              <button role="menuitem" onclick={() => act(openInFileManager)} disabled={!current.mounted}>
                Open in file manager
              </button>
              {#if current.mounted}
                <button role="menuitem" onclick={() => act(eject)}>Eject (keeps sharing)</button>
              {:else}
                <button role="menuitem" onclick={() => act(mount)}>Mount</button>
              {/if}
            </div>
          {/if}
        </div>
        <StickMenu {sharing} bind:open={() => openMenu === "more", (open) => (openMenu = open ? "more" : null)} onPick={picked} />
      </div>
    {/if}
  </header>

  {#if asking && current}
    <StickAsk {stickId} name={current.name} ask={asking} onDone={asked} />
  {/if}

  <!-- Dropped onto, or clicked: the system's picker, for files or for
       folders; either way it is added as a drop is. -->
  <div class="menu-box drop-box">
    <button
      class="drop"
      class:dropping
      class:busy={drops.copying}
      aria-haspopup="menu"
      aria-expanded={openMenu === "add"}
      onclick={() => (openMenu = openMenu === "add" ? null : "add")}
    >
      {#if drops.copying}
        <strong>Copying…{drops.copied ? ` ${size(drops.copied.done)} of ${size(drops.copied.total)}` : ""}</strong>
        <progress max={drops.copied?.total || 1} value={drops.copied?.done ?? 0}></progress>
      {:else}
        <strong>Drop files or folders here, or click to choose</strong>
        <span class="muted">{contents(current?.entries ?? [])} · {size(current?.used ?? 0)}</span>
      {/if}
    </button>
    {#if openMenu === "add"}
      <div class="menu add-menu" role="menu">
        <button role="menuitem" onclick={() => choose("files")}>Add files…</button>
        <button role="menuitem" onclick={() => choose("folders")}>Add folders…</button>
      </div>
    {/if}
  </div>

  {#if drops.queue.length > 0}
    <p class="row spread queued">
      <span class="muted">Next: {#each queuedNames(drops) as name, index (index)}{index > 0 ? ", " : ""}<bdi>{name}</bdi>{/each}</span>
      <button onclick={() => cancelQueued(stickId)}>Don't add these</button>
    </p>
  {/if}

  {#if drops.alreadyThere}
    <AlreadyThere names={drops.alreadyThere.names} onAnswer={(choice) => answerAlreadyThere(stickId, choice)} />
  {/if}

  {#if drops.tooBig}
    <DoesNotFit {stickId} {...drops.tooBig} onClose={tooBigClosed} />
  {/if}

  <EntryList {stickId} entries={current?.entries ?? []} onChanged={refresh} onFailure={(why) => (failure = why)} />

  <section class="capacity">
    <span>{size(current?.used ?? 0)} used · {size(current?.left ?? 0)} left</span>
    <div class="bar"><div class="fill" class:full={current?.left === 0} style="width: {fraction * 100}%"></div></div>
  </section>

  {#if activity.readers > 0}
    <p class="activity">
      ● {activity.readers === 1 ? "Someone downloading" : `${activity.readers} downloading`} · {activity.preparing ? "getting their files ready" : rate(activity.bytes_per_second)}
    </p>
    {#if activity.giving_way}
      <p class="muted">Sending slowly, so another program can use the disk.</p>
    {/if}
  {/if}
  <ReaderList {stickId} />
  {#if activity.local_network_declined}
    <LocalNetworkNote other="someone downloading" />
  {:else if activity.relay_only}
    <p class="error">{activity.relay_only}</p>
  {/if}

  {#if current?.sharing.state === "off"}
    <p class="row spread">
      <span class="muted">Not sharing: nobody can download.</span>
      <button onclick={() => act(startSharingAgain)}>Start sharing</button>
    </p>
  {:else if current?.sharing.state === "starting"}
    <p class="muted">Getting ready to share…</p>
  {:else if current?.sharing.state === "failed"}
    <p class="error">Not sharing: {current.sharing.message}</p>
  {/if}
  {#if givingWay && sharing}
    <p class="muted">Reading your folder slowly, so another program can use the disk.</p>
  {/if}
  {#if problem?.kind === "ejected"}
    <p class="muted">The stick is ejected; people still get its last version.</p>
  {:else if problem?.kind === "failed"}
    <p class="error">{problem.message}</p>
  {:else if problem?.kind === "seen_slowly"}
    <p class="muted">This stick has so many folders that changes in some of them show up within a minute, not at once.</p>
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
  {#each drops.failures as why, index (index)}<p class="error">{why}</p>{/each}

  <button class="share" onclick={() => openShare(stickId).catch((error) => (failure = String(error)))}>⤴ Share</button>
</section>

<style>
  .queued {
    margin: 0;
    gap: 8px;
  }
  .view {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
  }
  header h1 {
    margin: 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .drive {
    font-weight: 400;
  }
  .drop-box {
    flex-shrink: 0;
  }
  .drop {
    width: 100%;
    font: inherit;
    color: inherit;
    background: none;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    min-height: 110px;
    border: 2px dashed var(--border);
    border-radius: var(--radius);
  }
  .add-menu {
    left: 50%;
    right: auto;
    transform: translateX(-50%);
  }
  .drop.dropping {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .spread {
    justify-content: space-between;
  }
  .pill {
    border-radius: 999px;
    white-space: nowrap;
  }
  .pill .on {
    color: #2a9d4b;
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
