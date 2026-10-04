// What is being dropped onto each stick (gui-spec, the drop zone): one copy
// at a time; a drop meanwhile waits in its stick's queue and is copied once
// the one before is done, in the order dropped. A drop that asks something
// ("already on the stick", "does not fit") holds the queue until answered.
// Kept per stick for as long as the app runs, so another stick's view and
// back finds it as it was; a Quit drops what has not started.
import { listen } from "@tauri-apps/api/event";
import { addPaths, type CopyProgress } from "./api";

export interface Drops {
  copying: boolean;
  /** How far the copy is, while it runs. */
  copied: CopyProgress | null;
  /** Drops waiting for the copy before them, in order. */
  queue: string[][];
  /** A drop whose names are already on the stick, asked about. */
  alreadyThere: { paths: string[]; names: string[] } | null;
  /** A drop that does not fit, offered to be shared as it is. */
  tooBig: { paths: string[]; needed: number; left: number } | null;
  /** What went wrong since the user last dropped onto an idle stick, each
   * with the drop it was: kept while the queue goes on, so none is missed. */
  failures: string[];
}

const all = $state<Record<string, Drops>>({});

/** Nothing dropped yet: what a stick never dropped onto shows. */
const NONE: Drops = Object.freeze({
  copying: false,
  copied: null,
  queue: [],
  alreadyThere: null,
  tooBig: null,
  failures: [],
}) as Drops;

/** The stick's drops, for showing (never changes anything). */
export function dropsOf(stickId: string): Drops {
  return all[stickId] ?? NONE;
}

function of(stickId: string): Drops {
  all[stickId] ??= { ...NONE, queue: [], failures: [] };
  return all[stickId];
}

void listen<CopyProgress & { stick_id: string }>("copy-progress", (event) => {
  const drops = all[event.payload.stick_id];
  if (drops?.copying) drops.copied = event.payload;
});

/** Whether the stick waits on something before its next drop. */
function held(drops: Drops): boolean {
  return drops.copying || drops.alreadyThere !== null || drops.tooBig !== null;
}

/** Copies `paths` onto the stick now, or after what is before them. */
export async function dropOn(
  stickId: string,
  paths: string[],
  onConflict: "replace" | "keep_both" | null = null,
  fromQueue = false,
): Promise<void> {
  if (paths.length === 0) return;
  const drops = of(stickId);
  if (held(drops) && onConflict === null) {
    drops.queue.push(paths);
    return;
  }
  // A new start (not one from the queue): what failed before is old news.
  if (!fromQueue && drops.queue.length === 0) drops.failures = [];
  drops.copying = true;
  drops.copied = null;
  try {
    const outcome = await addPaths(stickId, paths, onConflict);
    if (outcome.kind === "does_not_fit") {
      drops.tooBig = { paths, needed: outcome.needed, left: outcome.left };
    } else if (outcome.kind === "already_there") {
      drops.alreadyThere = { paths, names: outcome.names };
    }
  } catch (error) {
    drops.failures.push(`${namesOf(paths).join(", ")}: ${String(error)}`);
  } finally {
    drops.copying = false;
    drops.copied = null;
  }
  next(stickId);
}

/** The next drop in the queue, if nothing holds it. */
function next(stickId: string) {
  const drops = of(stickId);
  if (held(drops)) return;
  const paths = drops.queue.shift();
  if (paths) void dropOn(stickId, paths, null, true);
}

/** The answer to "already on the stick"; none cancels that drop only. */
export function answerAlreadyThere(
  stickId: string,
  choice: "replace" | "keep_both" | null,
) {
  const drops = of(stickId);
  const asked = drops.alreadyThere;
  drops.alreadyThere = null;
  if (asked && choice) void dropOn(stickId, asked.paths, choice, true);
  else next(stickId);
}

/** "Does not fit" closed (shared as it is, or not); `why` if it failed. */
export function closeTooBig(stickId: string, why: string | null) {
  const drops = of(stickId);
  const asked = drops.tooBig;
  drops.tooBig = null;
  if (why)
    drops.failures.push(
      asked ? `${namesOf(asked.paths).join(", ")}: ${why}` : why,
    );
  next(stickId);
}

/** Drops the waiting ones: what is being copied goes on. */
export function cancelQueued(stickId: string) {
  of(stickId).queue = [];
}

/** The names a drop shows: each path's last part. */
function namesOf(paths: string[]): string[] {
  return paths.map((path) => path.split(/[\\/]/).filter(Boolean).pop() ?? path);
}

/** The names a queue line shows. */
export function queuedNames(drops: Drops): string[] {
  return drops.queue.flatMap(namesOf);
}
