<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    appProblem,
    dismissAppProblem,
    sticks,
    type About,
    type Activity,
    type ShareProblem,
    type StickEntry,
  } from "./api";
  import StickList from "./StickList.svelte";
  import StickView from "./StickView.svelte";
  import type { Outcome } from "./StickAsk.svelte";

  /** The app's sticks (ADR 0009), oldest first; one is shown. */
  let entries = $state<StickEntry[]>([]);
  let selected = $state<string | null>(null);
  let creating = $state(false);
  let failure = $state<string | null>(null);
  /** A problem of the whole app, not of one stick. */
  let problem = $state<ShareProblem | null>(null);
  /** Answers to an older look are dropped: they arrive in any order. */
  let looks = 0;

  async function refresh() {
    const look = ++looks;
    try {
      const [loaded, appWide] = await Promise.all([sticks(), appProblem()]);
      if (look !== looks) return;
      entries = loaded;
      problem = appWide;
      failure = null;
      // The one shown stays, if it is still there; else the first.
      if (!entries.some((entry) => entry.id === selected)) {
        selected = entries[0]?.id ?? null;
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
    selected = stickId;
    await refresh();
  }

  function followed(outcome: Outcome) {
    // A new link is a new stick id: the view follows it.
    if (outcome?.kind === "new-link") selected = outcome.id;
    refresh();
  }

  onMount(() => {
    refresh();
    const stops = [
      listen("sticks-changed", refresh),
      listen("sharing-changed", refresh),
      listen("mount-changed", refresh),
      listen("problems-changed", refresh),
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
    <StickList {entries} {selected} bind:creating onSelect={select} />
    {#if selected}
      {#key selected}
        <StickView stickId={selected} onOutcome={followed} />
      {/key}
    {:else if failure}
      <p class="error">{failure}</p>
    {:else}
      <p class="muted empty">No stick yet: make one with “New stick…”.</p>
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
