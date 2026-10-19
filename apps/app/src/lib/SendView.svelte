<script lang="ts">
  // Send, step 1: "What do you want to send?" (gui-spec, Send). Everything
  // chosen is checked by the core's one place, counted, then sent from
  // where it is; nothing is copied.
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { chooseForStick } from "./api";
  import { checkSend, measure, send, type Measured, type Verdict } from "./sendApi";
  import { size } from "./format";
  import AskFirst from "./AskFirst.svelte";

  let { onBack, onSent }: { onBack: () => void; onSent: (stickId: string) => void } = $props();

  /** One line of the list: chosen (with its count), or refused in its place. */
  interface Line {
    /** Its place in the list: the same path may be there refused too. */
    id: number;
    verdict: Verdict;
    counted: Measured | null;
    soFar: number;
  }

  let lines = $state<Line[]>([]);
  let lineIds = 0;
  /** Asked about one at a time, in the order chosen. */
  let asking = $state<Verdict[]>([]);
  let name = $state("");
  /** The name was typed, not filled in from the first thing chosen. */
  let named = $state(false);
  let dropping = $state(false);
  let busy = $state(false);
  let failure = $state<string | null>(null);

  let chosen = $derived(lines.filter((line) => line.verdict.kind !== "refuse"));

  function take(verdict: Verdict) {
    lines.push({ id: ++lineIds, verdict, counted: null, soFar: 0 });
    const shown = lines[lines.length - 1];
    if (!named && chosen.length === 1) name = verdict.name;
    measure(verdict.path, (measured) => (shown.soFar = measured.files))
      .then((measured) => (shown.counted = measured))
      .catch(() => {});
  }

  async function add(paths: string[]) {
    if (paths.length === 0) return;
    failure = null;
    try {
      const verdicts = await checkSend(
        paths,
        chosen.map((line) => line.verdict.path),
      );
      for (const verdict of verdicts) {
        if (verdict.kind === "ask") asking.push(verdict);
        else if (verdict.kind === "fine") take(verdict);
        else lines.push({ id: ++lineIds, verdict, counted: null, soFar: 0 });
      }
    } catch (error) {
      failure = String(error);
    }
  }

  async function answer(all: boolean) {
    const verdict = asking.shift();
    if (!verdict) return;
    if (all) {
      // Checked again with what was chosen meanwhile.
      const [again] = await checkSend([verdict.path], chosen.map((line) => line.verdict.path)).catch(() => [null]);
      if (again && again.kind !== "refuse") take(again);
      else if (again) lines.push({ id: ++lineIds, verdict: again, counted: null, soFar: 0 });
    } else if (asking.length === 0) {
      await pick("folders");
    }
  }

  async function pick(what: "files" | "folders") {
    try {
      await add(await chooseForStick(what));
    } catch (error) {
      failure = String(error);
    }
  }

  function remove(line: Line) {
    lines = lines.filter((other) => other !== line);
    if (!named) name = chosen[0]?.verdict.name ?? "";
  }

  async function getLink(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    failure = null;
    try {
      onSent(
        await send(
          name,
          chosen.map((line) => line.verdict.path),
        ),
      );
    } catch (error) {
      failure = String(error);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    const stop = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "over" || event.payload.type === "enter") {
        dropping = true;
      } else {
        dropping = false;
        if (event.payload.type === "drop") void add(event.payload.paths);
      }
    });
    return () => void stop.then((unlisten) => unlisten());
  });

  function counting(line: Line): string {
    if (line.counted) return size(line.counted.bytes);
    return line.soFar > 0 ? `counting… ${line.soFar.toLocaleString()} files so far` : "counting…";
  }
</script>

<section class="view">
  <button class="back" onclick={onBack}>‹ Back</button>
  <h1>What do you want to send?</h1>

  <div class="drop" class:dropping>
    <span>{lines.length > 0 ? "Drag more here, or" : "Drag folders or files here, or"}</span>
    <button onclick={() => pick("folders")} disabled={busy}>Choose folders…</button>
    <button onclick={() => pick("files")} disabled={busy}>Choose files…</button>
  </div>

  {#if lines.length > 0}
    <ul class="chosen">
      {#each lines as line (line.id)}
        <li class:refused={line.verdict.kind === "refuse"}>
          <span class="icon" aria-hidden="true">{line.verdict.kind === "refuse" ? "!" : line.verdict.is_dir ? "📁" : "📄"}</span>
          {#if line.verdict.kind === "refuse"}
            <span class="why error">{line.verdict.why}</span>
          {:else}
            <bdi class="name">{line.verdict.name}</bdi>
            <span class="size muted">{counting(line)}</span>
          {/if}
          <button class="remove" aria-label="Remove {line.verdict.name}" onclick={() => remove(line)}>×</button>
        </li>
      {/each}
    </ul>
    <p class="nothing-copied">
      Nothing is copied: OurStick sends them from where they are. <b>Leave them there.</b> If you change a file, they get
      the change too.
    </p>
  {/if}

  <form onsubmit={getLink}>
    <label>
      Name, as others will see it
      <input bind:value={name} oninput={() => (named = true)} maxlength="60" required disabled={busy} />
    </label>
    <button class="primary" type="submit" disabled={busy || chosen.length === 0 || !name.trim()}>Get the link</button>
  </form>
  {#if chosen.length > 0 && !named}<p class="muted hint">Filled in from the first folder. You can change it.</p>{/if}
  {#if busy}<p class="muted">Getting it ready…</p>{/if}
  {#if failure}<p class="error">{failure}</p>{/if}
</section>

{#if asking.length > 0}
  {#key asking[0].path}
    <AskFirst verdict={asking[0]} onAnswer={answer} />
  {/key}
{/if}

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
    overflow-y: auto;
  }
  .back {
    align-self: flex-start;
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
  }
  h1 {
    margin: 0;
  }
  .drop {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 18px;
    border: 2px dashed var(--border);
    border-radius: var(--radius);
  }
  .drop.dropping {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, var(--card));
  }
  .chosen {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .chosen li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .chosen li.refused {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  }
  .name,
  .why {
    flex: 1 1 auto;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .remove {
    padding: 0 10px;
  }
  .nothing-copied {
    margin: 0;
    padding: 10px 12px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--accent) 8%, var(--card));
  }
  form {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 10px;
  }
  label {
    flex: 1 1 240px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .hint {
    margin: -6px 0 0;
    font-size: 0.9em;
  }
</style>
