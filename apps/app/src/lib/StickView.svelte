<script lang="ts">
  // Something you send (gui-spec): its name, Copy link with the amber line,
  // ⋯; who has it; its notes; what is in it, with `+ Add more`. Dropped onto
  // the window, things are added as with `+ Add more`.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import {
    copyLink,
    linkPaths,
    openDialog,
    openInFileManager,
    startSharingAgain,
    stick as loadStick,
    type About,
    type Activity,
    type ShareProblem,
    type Stick,
  } from "./api";
  import { checkSend, choosePaths, type Verdict } from "./sendApi";
  import { rate } from "./format";
  import AskFirst from "./AskFirst.svelte";
  import EntryList from "./EntryList.svelte";
  import ReaderList from "./ReaderList.svelte";
  import LocalNetworkNote from "./LocalNetworkNote.svelte";
  import StickMenu, { type Pick } from "./StickMenu.svelte";
  import StickAsk, { type Ask, type Outcome } from "./StickAsk.svelte";

  let {
    stickId,
    onOutcome,
  }: {
    stickId: string;
    /** A rename, new link or removal done here: the window follows. */
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
  let failure = $state<string | null>(null);
  /** What `+ Add more` refused, each in its words. */
  let refused = $state<string[]>([]);
  let menuOpen = $state(false);
  let addOpen = $state(false);
  let asking = $state<Ask | null>(null);
  /** Asked about before being added, one at a time. */
  let guarding = $state<Verdict[]>([]);
  let dropping = $state(false);
  let copied = $state(false);
  /** OurStick gives way to another program reading the same disk. */
  let givingWay = $state(false);

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

  async function act(action: (stickId: string) => Promise<void>) {
    failure = null;
    try {
      await action(stickId);
    } catch (error) {
      failure = String(error);
    }
    await refresh();
  }

  async function copy() {
    failure = null;
    try {
      await copyLink(stickId);
      copied = true;
    } catch (error) {
      failure = String(error);
    }
  }

  function picked(pick: Pick) {
    if (pick === "stop") act((id) => openDialog(false, id));
    else if (pick === "start") act(startSharingAgain);
    else if (pick === "show-files") act(openInFileManager);
    else asking = pick;
  }

  function asked(outcome: Outcome) {
    asking = null;
    onOutcome(outcome);
    if (outcome?.kind === "renamed") refresh();
  }

  /** `+ Add more`: checked by the same rules as Send; refused ones said,
   * a main folder or a whole disk asked about first, the rest added. */
  async function add(paths: string[]) {
    if (paths.length === 0) return;
    failure = null;
    refused = [];
    try {
      const verdicts = await checkSend(paths, [], stickId);
      refused = verdicts.filter((verdict) => verdict.kind === "refuse").map((verdict) => verdict.why ?? "");
      guarding.push(...verdicts.filter((verdict) => verdict.kind === "ask"));
      const fine = verdicts.filter((verdict) => verdict.kind === "fine").map((verdict) => verdict.path);
      if (fine.length > 0) await linkPaths(stickId, fine);
    } catch (error) {
      failure = String(error);
    }
    await refresh();
  }

  async function guarded(all: boolean) {
    const verdict = guarding.shift();
    if (!verdict) return;
    if (all) {
      await linkPaths(stickId, [verdict.path]).catch((error) => (failure = String(error)));
      await refresh();
    } else if (guarding.length === 0) {
      await choose("folders");
    }
  }

  async function choose(pick: "files" | "folders") {
    addOpen = false;
    try {
      await add(await choosePaths(pick));
    } catch (error) {
      failure = String(error);
    }
  }

  /** A click outside a menu, or Escape, closes it. */
  function closeMenus(event: MouseEvent | KeyboardEvent) {
    if (event instanceof KeyboardEvent) {
      if (event.key === "Escape") menuOpen = addOpen = false;
    } else if (!(event.target instanceof Element && event.target.closest(".menu-box"))) {
      menuOpen = addOpen = false;
    }
  }

  onMount(() => {
    refresh();
    const stops = [
      listen("stick-changed", refresh),
      listen("sharing-changed", refresh),
      listen("mount-changed", refresh),
      listen("problems-changed", refresh),
      // Only this one's.
      listen<About<Activity>>("activity", (event) => {
        if (event.payload.stick_id === stickId) activity = event.payload;
      }),
      // Told at each change only (ADR 0013).
      listen<About<{ giving_way: boolean }>>("giving-way", (event) => {
        if (event.payload.stick_id === stickId) givingWay = event.payload.giving_way;
      }),
      getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "over" || event.payload.type === "enter") {
          dropping = true;
        } else {
          dropping = false;
          if (event.payload.type === "drop") void add(event.payload.paths);
        }
      }),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  });

  let shareState = $derived(current?.sharing.state ?? "off");
  let sending = $derived(shareState === "on" || shareState === "starting");
  let problem = $derived<ShareProblem | null>(current?.problem ?? null);
  /** The one folder it was made from, if its contents are at the top. */
  let top = $derived(current?.entries.find((entry) => entry.at_top) ?? null);
</script>

<svelte:window onclick={closeMenus} onkeydown={closeMenus} />

<section class="view" class:dropping>
  {#if current}
    <header class="row">
      <h1 title={current.name}><bdi>{current.name}</bdi></h1>
      <button class="primary" onclick={copy} disabled={!current.sharing.has_link}>{copied ? "Copied ✓" : "Copy link"}</button>
      <StickMenu {sending} ownFolder={current.own_folder} bind:open={menuOpen} onPick={picked} />
    </header>
    <p class="warning">Anyone who gets the link can take everything in it, also what you add later.</p>

    <!-- Who has it (ADR 0015), and how it goes now. -->
    <div class="status">
      {#if activity.readers > 0}
        <p>
          ↑ {activity.readers === 1 ? "Someone is getting it" : `${activity.readers} people are getting it`}
          <span class="muted">· {activity.preparing ? "preparing their files first…" : rate(activity.bytes_per_second)}</span>
        </p>
      {/if}
      <ReaderList {stickId} nobodyNow={activity.readers === 0} />
    </div>

    <!-- Notes, each while it holds. -->
    {#if shareState === "off"}
      <div class="note row spread">
        <span><b>⏸ Stopped by you.</b> The link does not work until you start again. Nothing was deleted.</span>
        <button onclick={() => act(startSharingAgain)}>Resume</button>
      </div>
    {:else if shareState === "starting"}
      <p class="muted">Getting it ready…</p>
    {:else if shareState === "failed"}
      <div class="note row spread error">
        <span><b>✕ Stopped:</b> {current.sharing.message}</span>
        <button onclick={() => act(startSharingAgain)}>Try again</button>
      </div>
    {/if}
    {#if activity.giving_way && activity.readers > 0}
      <p class="muted">Sending slowly, so another program can use the disk.</p>
    {/if}
    {#if givingWay && sending}
      <p class="muted">Reading your folders slowly, so another program can use the disk.</p>
    {/if}
    {#if activity.local_network_declined}
      <LocalNetworkNote other="someone getting it" />
    {:else if activity.relay_only}
      <p class="warning">{activity.relay_only}</p>
    {/if}
    {#if problem?.kind === "failed"}
      <p class="error">{problem.message}</p>
    {:else if problem?.kind === "seen_slowly"}
      <p class="muted">Changes in some folders reach the others within a minute.</p>
    {/if}
    {#if current.own_folder && !current.folder_there}
      <p class="warning">Some of its files are on a disk that is not attached.</p>
    {/if}

    <div class="in-it">
      <div class="row spread">
        <h2>What is in it</h2>
        <div class="menu-box">
          <button aria-haspopup="menu" aria-expanded={addOpen} onclick={() => (addOpen = !addOpen)}>+ Add more</button>
          {#if addOpen}
            <div class="menu" role="menu">
              <button role="menuitem" onclick={() => choose("folders")}>Choose folders…</button>
              <button role="menuitem" onclick={() => choose("files")}>Choose files…</button>
              {#if top}<p class="hint">They go next to what is in “<bdi>{top.name}</bdi>”.</p>{/if}
            </div>
          {/if}
        </div>
      </div>
      {#each refused as why, index (index)}<p class="error">{why}</p>{/each}
      <EntryList {stickId} entries={current.entries} onChanged={refresh} onFailure={(why) => (failure = why)} />
      <p class="muted">
        Leave them where they are. If you change a file, they get the change too. If you move or delete one, it stops
        coming; what they have stays with them.
      </p>
    </div>
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
</section>

{#if asking && current}
  <StickAsk {stickId} name={current.name} ask={asking} folder={current.folder} onDone={asked} />
{/if}
{#if guarding.length > 0}
  {#key guarding[0].path}
    <AskFirst verdict={guarding[0]} onAnswer={guarded} />
  {/key}
{/if}

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
    overflow-y: auto;
    border-radius: var(--radius);
  }
  .view.dropping {
    outline: 2px dashed var(--accent);
    outline-offset: 4px;
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
  .status {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .note {
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .spread {
    justify-content: space-between;
  }
  .in-it {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h2 {
    margin: 0;
    font-size: 1.05em;
  }
  .hint {
    margin: 0;
    padding: 6px 12px 8px;
    max-width: 16em;
    font-size: 0.85em;
    color: var(--muted);
  }
  p {
    margin: 0;
  }
</style>
