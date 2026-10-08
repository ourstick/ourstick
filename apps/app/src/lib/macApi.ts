// The window's commands for the Local Network step and message (ADR 0020)
// and the link already on the clipboard.
import { invoke } from "@tauri-apps/api/core";

/** The clipboard's text, if it is a whole sharing link (else null). */
export function linkOnClipboard(): Promise<string | null> {
  return invoke("link_on_clipboard");
}

/** "Continue" on the Local Network step: the sticks start, and macOS asks. */
export function localNetworkExplained(): Promise<void> {
  return invoke("local_network_explained");
}

/** Opens System Settings → Privacy & Security → Local Network (macOS). */
export function openLocalNetworkSettings(): Promise<void> {
  return invoke("open_local_network_settings");
}
