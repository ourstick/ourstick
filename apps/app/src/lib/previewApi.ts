// A pasted sharing link: its preview, waiting for its sender (ADR 0025),
// and adding it. Types mirror src-tauri/src/receiving/preview.rs.
import { invoke } from "@tauri-apps/api/core";
import type { SavedAsList } from "./api";

export interface PreviewEntry {
  /** Exactly as the stick names it, its whole path: send this back. */
  name: string;
  /** Its last part, escaped: show this (isolated). */
  shown: string;
  is_dir: boolean;
  files: number;
  bytes: number;
  /** What it would write into the folder (files already there left out). */
  to_write: number;
}

/** What adding a pasted sharing link would bring. */
export interface PreviewView {
  name: string;
  /** The sender is still counting its files. */
  partial: boolean;
  entries: PreviewEntry[];
  total: number;
  to_write: number;
  folder: string;
  /** Received there earlier with the command line: it goes on there. */
  earlier: boolean;
  /** Received earlier in another folder: it moves; the files there stay. */
  earlier_elsewhere: string | null;
  /** What is left on this computer where it would go. */
  left: number;
  /** What would be saved under another name there. */
  saved_as: SavedAsList;
}

/** A preview, or the sender away (then it can be waited for). */
export type PreviewOutcome = { kind: "seen"; preview: PreviewView } | { kind: "sender_away" };

/** Looks at a pasted sharing link: `parent` is where it would be saved (else Downloads). */
export function previewLink(
  text: string,
  parent: string | null,
): Promise<PreviewOutcome> {
  return invoke("preview_link", { text, parent });
}

/** "Wait for it": kept, and received by itself, everything ticked, once
 * its sender is back. Returns its id. */
export function waitForLink(text: string): Promise<string> {
  return invoke("wait_for_link", { text });
}

/** The system's folder picker; null if cancelled. */
export function chooseFolder(): Promise<string | null> {
  return invoke("choose_folder");
}

/** What is inside a top-level folder of the latest preview, each named by
 * its whole path. */
export function previewInside(folder: string): Promise<PreviewEntry[]> {
  return invoke("preview_inside", { folder });
}

/** What the latest preview would write with `leftOut` not got. */
export function previewNeeds(leftOut: string[]): Promise<number> {
  return invoke("preview_needs", { leftOut });
}

/** "Get it": received into `folder` and kept up to date, but for the
 * paths `leftOut` (exactly as the stick names them). Returns its id. */
export function addReceived(
  text: string,
  folder: string,
  name: string,
  leftOut: string[] = [],
): Promise<string> {
  return invoke("add_received", { text, folder, name, leftOut });
}
