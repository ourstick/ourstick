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

/** What opened Receive from outside, given once (else null): a link opened
 * from another app (a phone's), or `ourstick:receive` or a new link found
 * on coming back (ADR 0030), which Receive asks about before it looks. */
export type Opened = { kind: "link"; link: string } | { kind: "asked" };

export function takeOpenedLink(): Promise<Opened | null> {
  return invoke("take_opened_link");
}

/** The window came back to Home: whether a link not seen before is on the
 * clipboard (ADR 0030 §4); Receive then waits for the window. */
export function newLinkOnClipboard(): Promise<boolean> {
  return invoke("new_link_on_clipboard");
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
