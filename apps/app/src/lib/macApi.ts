// The window's commands for the Local Network step and message (ADR 0020)
// and the link already on the clipboard or opened from another app.
import { invoke } from "@tauri-apps/api/core";

/** The clipboard's text, if it is a whole sharing link (else null). */
export function linkOnClipboard(): Promise<string | null> {
  return invoke("link_on_clipboard");
}

/** Whether a link opened from another app (a phone's) waits for Receive. */
export function linkOpened(): Promise<boolean> {
  return invoke("link_opened");
}

/** The link opened from another app, given once (else null). */
export function takeOpenedLink(): Promise<string | null> {
  return invoke("take_opened_link");
}

/** "Continue" on the Local Network step: the sticks start, and macOS asks. */
export function localNetworkExplained(): Promise<void> {
  return invoke("local_network_explained");
}

/** Opens System Settings → Privacy & Security → Local Network (macOS). */
export function openLocalNetworkSettings(): Promise<void> {
  return invoke("open_local_network_settings");
}

/** The keys' step (ADR 0021): move them now, or (`later`) not in this run. */
export function moveKeys(later: boolean): Promise<void> {
  return invoke("move_keys", { later });
}
