<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { sticks, type StickEntry } from "./api";
  import StickList from "./StickList.svelte";
  import StickView from "./StickView.svelte";
  import type { Outcome } from "./StickAsk.svelte";

  /** The app's sticks (ADR 0009), oldest first; one is shown. */
  let entries = $state<StickEntry[]>([]);
  let selected = $state<string | null>(null);
  let creating = $state(false);
  let failure = $state<string | null>(null);

  async function refresh() {
    try {
      entries = await sticks();
      failure = null;
      // The one shown stays, if it is still there; else the first.
      if (!entries.some((entry) => entry.id === selected)) {
        selected = entries[0]?.id ?? null;
      }
    } catch (error) {
      failure = String(error);
    }
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
      listen("activity", refresh),
      // "New stick…" from the tray.
      listen("new-stick", () => (creating = true)),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  });
</script>

<main class="card window">
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
</main>

<style>
  .window {
    flex-direction: row;
    gap: 16px;
  }
  .empty {
    margin: auto;
  }
</style>
