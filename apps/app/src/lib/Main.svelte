<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getVersion } from "@tauri-apps/api/app";
  import {
    appProblem,
    dismissAppProblem,
    received,
    sticks,
    type About,
    type Activity,
    type ReceivedEntry,
    type ShareProblem,
    type StickEntry,
  } from "./api";
  import AddLink from "./AddLink.svelte";
  import ReceivedList from "./ReceivedList.svelte";
  import ReceivedView from "./ReceivedView.svelte";
  import StickList from "./StickList.svelte";
  import StickView from "./StickView.svelte";
  import type { Outcome } from "./StickAsk.svelte";

  let { startAdding = false }: { startAdding?: boolean } = $props();

  /** The app's sticks (ADR 0009), oldest first, and those it receives. */
  let entries = $state<StickEntry[]>([]);
  let receivedEntries = $state<ReceivedEntry[]>([]);
  /** What the right pane shows. */
  type Shown = { kind: "own"; id: string } | { kind: "received"; id: string } | { kind: "add" } | null;
  // The window opened to receive (first launch, "I got a sharing link").
  // svelte-ignore state_referenced_locally
  let shown = $state<Shown>(startAdding ? { kind: "add" } : null);
  let selected = $derived(shown?.kind === "own" ? shown.id : null);
  let selectedReceived = $derived(shown?.kind === "received" ? shown.id : null);
  let receivedShown = $derived(receivedEntries.find((entry) => entry.id === selectedReceived) ?? null);
  let creating = $state(false);
  let failure = $state<string | null>(null);
  /** Shown at the bottom of the sidebar, so a report can say which one. */
  let version = $state("");
  /** A problem of the whole app, not of one stick. */
  let problem = $state<ShareProblem | null>(null);
  /** Answers to an older look are dropped: they arrive in any order. */
  let looks = 0;

  async function refresh() {
    const look = ++looks;
    try {
      const [loaded, coming, appWide] = await Promise.all([sticks(), received(), appProblem()]);
      if (look !== looks) return;
      entries = loaded;
      receivedEntries = coming;
      problem = appWide;
      failure = null;
      // What is shown stays, if it is still there; else the first stick.
      const now = shown;
      const there =
        now?.kind === "add" ||
        (now?.kind === "own" && entries.some((entry) => entry.id === now.id)) ||
        (now?.kind === "received" && receivedEntries.some((entry) => entry.id === now.id));
      if (!there) {
        const first = entries[0]?.id ?? null;
        const firstReceived = receivedEntries[0]?.id ?? null;
        shown = first ? { kind: "own", id: first } : firstReceived ? { kind: "received", id: firstReceived } : null;
      }
    } catch (error) {
      if (look === looks) failure = String(error);
    }
  }

  async function dismiss() {
    problem = null;
    await dismissAppProblem().catch((error) => (failure = String(error)));
  }

  async function select(stickId: string) {
    shown = { kind: "own", id: stickId };
    await refresh();
  }

  async function selectReceived(stickId: string) {
    shown = { kind: "received", id: stickId };
    await refresh();
  }

  function followed(outcome: Outcome) {
    // A new link is a new stick id: the view follows it.
    if (outcome?.kind === "new-link") shown = { kind: "own", id: outcome.id };
    refresh();
  }

  getVersion().then((v) => (version = v)).catch(() => {});
  onMount(() => {
    refresh();
    const stops = [
      listen("sticks-changed", refresh),
      listen("sharing-changed", refresh),
      listen("mount-changed", refresh),
      listen("problems-changed", refresh),
      listen("received-changed", refresh),
      // The list shows who downloads, not how fast: only a new count matters.
      listen<About<Activity>>("activity", (event) => {
        const shown = entries.find((entry) => entry.id === event.payload.stick_id);
        if (shown && shown.readers !== event.payload.readers) refresh();
      }),
      // "New stick…" from the tray.
      listen("new-stick", () => (creating = true)),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  });
</script>

<main class="card window">
  {#if problem?.kind === "failed"}
    <p class="app-problem error" role="alert">
      <span>{problem.message}</span>
      <button class="close" aria-label="Close" onclick={dismiss}>×</button>
    </p>
  {/if}
  <div class="panes">
    <div class="side">
      <StickList {entries} {selected} bind:creating onSelect={select} />
      <ReceivedList
        entries={receivedEntries}
        selected={selectedReceived}
        adding={shown?.kind === "add"}
        onSelect={selectReceived}
        onAdd={() => (shown = { kind: "add" })}
      />
      {#if version}<p class="version muted">OurStick {version}</p>{/if}
    </div>
    {#if shown?.kind === "own"}
      {#key shown.id}
        <StickView stickId={shown.id} onOutcome={followed} />
      {/key}
    {:else if shown?.kind === "received" && receivedShown}
      {#key shown.id}
        <ReceivedView entry={receivedShown} onForgotten={() => ((shown = null), refresh())} />
      {/key}
    {:else if shown?.kind === "add"}
      <AddLink onAdded={selectReceived} onCancel={() => ((shown = null), refresh())} />
    {:else if failure}
      <p class="error">{failure}</p>
    {:else}
      <p class="muted empty">No stick yet: make one with “New stick…”, or add a sharing link someone sent you.</p>
    {/if}
  </div>
</main>

<style>
  .window {
    gap: 12px;
  }
  .panes {
    display: flex;
    gap: 16px;
    flex: 1 1 auto;
    min-height: 0;
  }
  .side {
    width: 190px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
    padding-right: 12px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .version {
    margin: auto 0 0;
    font-size: 0.8em;
  }
  .app-problem {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    white-space: pre-line;
  }
  .close {
    padding: 0 8px;
  }
  .empty {
    margin: auto;
  }
</style>
