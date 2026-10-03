<script lang="ts" module>
  /** An entry the ticks list: at the top level, or inside a top-level folder
   * (named by its whole path). */
  export interface ChoiceRow {
    /** Exactly as the stick names it: what a choice is made with. */
    name: string;
    /** Its last part, as the window shows it (isolated). */
    shown: string;
    is_dir: boolean;
    bytes: number;
    /** Files of it are already here. */
    here?: boolean;
    /** On the stick now; false for a path left out the stick no longer has. */
    on_stick?: boolean;
  }
</script>

<script lang="ts">
  // The ticks as a tree (ADR 0012, Amendment 1): the top level, and what is
  // inside a top-level folder once it is opened. The core decides every
  // tick: the window keeps the set of paths left out and asks it.
  import { choiceTick, choiceTicks, type Tick } from "./api";
  import { size } from "./format";

  let {
    rows,
    leftOut = $bindable(),
    loadInside,
    disabled = false,
    known = true,
    none = $bindable(false),
    shownRows = $bindable([]),
    onFailure,
  }: {
    rows: ChoiceRow[];
    /** The paths left out, exactly as the stick names them. */
    leftOut: string[];
    /** What is inside a top-level folder; null when it cannot be asked. */
    loadInside: ((folder: string) => Promise<ChoiceRow[]>) | null;
    disabled?: boolean;
    /** The whole list is known (else only what is left out is listed). */
    known?: boolean;
    /** Every top-level entry is off: nothing would come. */
    none?: boolean;
    /** Every row shown, the inside of open folders too. */
    shownRows?: ChoiceRow[];
    onFailure: (why: string) => void;
  } = $props();

  let open = $state<Set<string>>(new Set());
  let inside = $state<Map<string, ChoiceRow[]>>(new Map());
  let ticks = $state<Record<string, Tick>>({});
  let busy = $state(false);
  /** The rows the single folder was opened for: opened once per list. */
  let openedFor: ChoiceRow[] | null = null;

  let all = $derived(rows.flatMap((row) => [row, ...(open.has(row.name) ? (inside.get(row.name) ?? []) : [])]));

  $effect(() => {
    shownRows = all;
  });

  // A new list (another look at the link, another folder) forgets what was
  // opened and read of the last one; a stick of one folder is shown open.
  $effect(() => {
    if (rows === openedFor) return;
    openedFor = rows;
    open = new Set();
    inside = new Map();
    if (rows.length === 1 && rows[0].is_dir && loadInside) void toggle(rows[0].name);
  });

  // The ticks of every row shown, asked again whenever the set or the rows
  // change; only the latest answer counts.
  let asked = 0;
  $effect(() => {
    const paths = all.map((row) => row.name);
    const set = [...leftOut];
    const mine = ++asked;
    choiceTicks(set, paths)
      .then((answer) => {
        if (mine !== asked) return;
        const next: Record<string, Tick> = {};
        paths.forEach((path, index) => (next[path] = answer[index]));
        ticks = next;
        none = known && rows.length > 0 && rows.every((row) => next[row.name] === "off");
      })
      .catch((error) => onFailure(String(error)));
  });

  function tickOf(row: ChoiceRow): Tick {
    return ticks[row.name] ?? (leftOut.includes(row.name) ? "off" : "on");
  }

  async function toggle(folder: string) {
    if (open.has(folder)) {
      const next = new Set(open);
      next.delete(folder);
      open = next;
      return;
    }
    if (!loadInside) return;
    if (!inside.has(folder)) {
      try {
        const loaded = await loadInside(folder);
        inside = new Map(inside).set(folder, loaded);
      } catch (error) {
        onFailure(String(error));
        return;
      }
    }
    open = new Set(open).add(folder);
  }

  async function tick(row: ChoiceRow, on: boolean) {
    const slash = row.name.indexOf("/");
    const siblings = slash < 0 ? [] : (inside.get(row.name.slice(0, slash)) ?? []).map((sibling) => sibling.name);
    busy = true;
    try {
      leftOut = await choiceTick([...leftOut], row.name, on, siblings);
    } catch (error) {
      onFailure(String(error));
    } finally {
      busy = false;
    }
  }
</script>

<ul class="entries">
  {#each all as row (row.name)}
    {@const deeper = row.name.includes("/")}
    {@const state = tickOf(row)}
    <li class:deeper>
      <span class="tick">
        {#if !deeper && row.is_dir && loadInside && row.on_stick !== false}
          <button
            class="arrow"
            aria-expanded={open.has(row.name)}
            aria-label={open.has(row.name) ? "Hide what is inside" : "Show what is inside"}
            onclick={() => toggle(row.name)}>{open.has(row.name) ? "▾" : "▸"}</button
          >
        {:else if !deeper}
          <span class="arrow"></span>
        {/if}
        <label class="tick">
          <input
            type="checkbox"
            checked={state === "on"}
            indeterminate={state === "part"}
            onchange={(event) => tick(row, event.currentTarget.checked)}
            disabled={disabled || busy}
          />
          <bdi class="name">{row.shown}{row.is_dir ? "/" : ""}</bdi>
        </label>
      </span>
      {#if row.on_stick === false}
        {#if known}<span class="muted size">not on the stick now</span>{/if}
      {:else}
        <span class="muted size">{size(row.bytes)}</span>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
    min-height: 0;
    flex: 1 1 auto;
  }
  .entries li {
    flex-shrink: 0;
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--button-radius);
  }
  .entries li.deeper {
    margin-left: 2em;
  }
  .tick {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .arrow {
    width: 1.6em;
    flex-shrink: 0;
    padding: 0;
    border: none;
    background: none;
    color: inherit;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    unicode-bidi: isolate;
  }
  .size {
    flex-shrink: 0;
    white-space: nowrap;
  }
  .muted {
    color: var(--muted);
  }
</style>
