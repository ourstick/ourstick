<script lang="ts">
  // Who has a shared stick (ADR 0015): one line per computer that received
  // it, in its receiver's own words, never a proof.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { KINDS, readers, removeReaders, type ReaderLine } from "./api";

  /** `nobodyNow`: no computer is getting it right now either. */
  let { stickId, nobodyNow = false }: { stickId: string; nobodyNow?: boolean } = $props();

  /** At most this many lines, then "and N more". */
  const SHOWN = 5;

  let lines = $state<ReaderLine[]>([]);
  let failure = $state<string | null>(null);

  async function load() {
    try {
      lines = await readers(stickId);
      failure = null;
    } catch (error) {
      failure = String(error);
    }
  }

  /** Many reports come at once (each reader every 30 s): the list is read
   * again at most four times a second. */
  let pending: ReturnType<typeof setTimeout> | null = null;
  function soon() {
    pending ??= setTimeout(() => {
      pending = null;
      void load();
    }, 250);
  }

  onMount(() => {
    // Listening first: a report that comes while the list loads is not
    // missed.
    const stop = listen<{ stick_id: string }>("readers-changed", (event) => {
      if (event.payload.stick_id === stickId) soon();
    });
    void stop.then(() => load());
    return () => {
      if (pending) clearTimeout(pending);
      void stop.then((unlisten) => unlisten());
    };
  });

  /** The list is read again when the change is told. */
  async function remove(reader: string | null) {
    try {
      await removeReaders(stickId, reader);
    } catch (error) {
      failure = String(error);
    }
  }

  function day(ms: number): string {
    return new Date(ms).toLocaleDateString([], { day: "numeric", month: "short" });
  }

  function at(ms: number): string {
    const when = new Date(ms);
    return new Date().toDateString() === when.toDateString()
      ? `at ${when.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
      : `on ${day(ms)}`;
  }

  /** The computer as the line names it: its kind, a name only if its
   * receiver typed one; two of a kind with no name told apart by when they
   * were first heard. */
  function who(line: ReaderLine): string {
    const kind = KINDS[line.kind];
    if (line.name) return `${line.name} (${kind})`;
    const twin = lines.some((other) => other !== line && other.kind === line.kind && !other.name);
    const named = `A ${kind}`;
    return twin ? `${named} (since ${day(line.first_ms)})` : named;
  }

  function what(line: ReaderLine): string {
    const all = line.everything ? "all of it" : "what it chose";
    switch (line.has) {
      case "all":
        return "has all of it, up to date.";
      case "what_it_chose":
        return "has what it chose, up to date.";
      case "so_far":
        return `has ${line.everything ? "everything" : "what it chose"} so far; you are still reading your folder.`;
      case "older":
        return `had ${all} ${at(line.received_ms)}; not what changed since.`;
      case "downloading":
        return `is downloading: ${line.percent ?? 0}%.`;
      case "paused":
        return "paused it.";
      case "waiting":
        switch (line.reason) {
          case "no_room":
            return "is waiting: no room there.";
          case "folder_missing":
            return "is waiting: its folder is not there.";
          case "sender_away":
            return "was waiting for this computer.";
          default:
            return "is waiting.";
        }
    }
  }

  let everyone = $derived(
    lines.length > 0 && lines.every((line) => line.has === "all" || line.has === "what_it_chose"),
  );
</script>

<!-- While no computer has said it has it, and none is getting it, one
     sentence says so. -->
<section class="readers" aria-label="Who has it" hidden={lines.length === 0 && !failure && !nobodyNow}>
  {#if lines.length > 0}
    {#if everyone}<p><strong>Everyone who received it is up to date.</strong></p>{/if}
    <ul>
      {#each lines.slice(0, SHOWN) as line (line.reader)}
        <li>
          <span><bdi>{who(line)}</bdi> {what(line)}</span>
          <button class="remove" aria-label="Remove from this list" title="Remove from this list" onclick={() => remove(line.reader)}
            >×</button
          >
        </li>
      {/each}
    </ul>
    <p class="row">
      {#if lines.length > SHOWN}<span class="muted">and {lines.length - SHOWN} more</span>{/if}
      <button onclick={() => remove(null)}>Clear this list</button>
    </p>
    <p class="muted">Each computer says this about itself.</p>
  {:else if nobodyNow}
    <p class="muted">Nobody has opened the link yet.</p>
  {/if}
  {#if failure}<p class="error">{failure}</p>{/if}
</section>

<style>
  .readers {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }
  .remove {
    padding: 0 6px;
    border: none;
    background: none;
    color: var(--muted);
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  p {
    margin: 0;
  }
  .muted {
    color: var(--muted);
  }
</style>
