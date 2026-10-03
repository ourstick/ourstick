// Choosing what to get of a received stick (ADR 0012 and its Amendment 1).
import { invoke } from "@tauri-apps/api/core";

/** How an entry shows in the ticks: a folder with some of its inside left
 * out is "part". */
export type Tick = "on" | "off" | "part";

/** A received stick's entry, at the top level or inside a top-level
 * folder, as the choice lists it. */
export interface ChoiceEntry {
  /** Exactly as the stick names it, its whole path: send this back. */
  name: string;
  /** Its last part, escaped: show this (isolated). */
  shown: string;
  is_dir: boolean;
  files: number;
  bytes: number;
  tick: Tick;
  /** Files of it are already here. */
  here: boolean;
  /** On the stick now (an un-ticked name may no longer be). */
  on_stick: boolean;
}

export interface ChoiceView {
  entries: ChoiceEntry[];
  /** Every entry is listed, not only those already left out. */
  known: boolean;
  /** Every path left out, shown or not: a save sends it all. */
  left_out: string[];
  /** Those of them whose files are already here. */
  staying: { name: string; shown: string }[];
}

/** "Choose what to get…": a received stick's top-level entries, or a
 * top-level `folder`'s. */
export function receivedContents(
  stickId: string,
  folder: string | null = null,
): Promise<ChoiceView> {
  return invoke("received_contents", { stickId, folder });
}

/** The set of paths left out after ticking `path` on or off; `siblings`
 * are the paths beside it in its folder. The core decides. */
export function choiceTick(
  leftOut: string[],
  path: string,
  on: boolean,
  siblings: string[],
): Promise<string[]> {
  return invoke("choice_tick", { leftOut, path, on, siblings });
}

/** How each of `paths` shows with `leftOut`. */
export function choiceTicks(
  leftOut: string[],
  paths: string[],
): Promise<Tick[]> {
  return invoke("choice_ticks", { leftOut, paths });
}

/** Saves what not to get (the whole set) and receives the stick again. */
export function chooseWhatToGet(
  stickId: string,
  leftOut: string[],
): Promise<void> {
  return invoke("choose_what_to_get", { stickId, leftOut });
}
