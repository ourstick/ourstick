// Send (ADR 0028 §2). Types mirror src-tauri/src/send.rs.
import { Channel, invoke } from "@tauri-apps/api/core";

/** What the window may do with one chosen path. */
export interface Verdict {
  path: string;
  name: string;
  is_dir: boolean;
  kind: "fine" | "ask" | "refuse";
  /** Refused: why, for the user. */
  why: string | null;
  /** Asked first: a main folder of the home, or a whole disk. */
  ask: "main" | "disk" | null;
  question: string | null;
}

/** Checks `paths`, each with `chosen` and those before it; for `+ Add more`
 * on `stickId`, with what it sends already too. */
export function checkSend(paths: string[], chosen: string[], stickId: string | null = null): Promise<Verdict[]> {
  return invoke("check_send", { paths, chosen, stickId });
}

export interface Measured {
  files: number;
  bytes: number;
}

/** Counts what is at `path`; `soFar` hears as it goes. */
export function measure(path: string, soFar: (measured: Measured) => void): Promise<Measured> {
  const channel = new Channel<Measured>();
  channel.onmessage = soFar;
  return invoke("measure", { path, soFar: channel });
}

/** "Get the link": a new thing sent, called `name`. Returns its id. */
export function send(name: string, paths: string[]): Promise<string> {
  return invoke("send", { name, paths });
}
