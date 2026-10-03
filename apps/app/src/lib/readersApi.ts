// Who has a shared stick, and the name senders see (ADR 0015).
import { invoke } from "@tauri-apps/api/core";

/** One computer that received a shared stick, as its line says it. */
export interface ReaderLine {
  /** Its id, for "Remove from this list". */
  reader: string;
  kind: "macos" | "windows" | "linux" | "other";
  /** A name its receiver typed (escaped); usually none. */
  name: string | null;
  has: "all" | "what_it_chose" | "so_far" | "older" | "downloading" | "waiting" | "paused";
  everything: boolean;
  percent: number | null;
  reason: "sender_away" | "no_room" | "folder_missing" | "other" | null;
  /** When this computer first and last heard from it. */
  first_ms: number;
  received_ms: number;
}

export function readers(stickId: string): Promise<ReaderLine[]> {
  return invoke("readers", { stickId });
}

/** "Remove from this list" for one line, or every line (`reader` null). */
export function removeReaders(stickId: string, reader: string | null): Promise<void> {
  return invoke("remove_readers", { stickId, reader });
}

/** How senders see this computer: its kind, and a name if the receiver
 * chose one. */
export function receiverName(): Promise<{ kind: ReaderLine["kind"]; name: string | null }> {
  return invoke("receiver_name");
}

/** A computer's kind as the windows say it. */
export const KINDS = { macos: "Mac", windows: "Windows PC", linux: "Linux PC", other: "computer" };

/** Sets the name senders see; empty clears it. */
export function setReceiverName(name: string): Promise<void> {
  return invoke("set_receiver_name", { name });
}
