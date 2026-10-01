// The only way the UI talks to the app: Tauri commands, which call only
// ourstick-core. Types mirror the Rust side (src-tauri/src/commands.rs).
import { invoke } from "@tauri-apps/api/core";

export type Platform = "linux" | "macos" | "windows";

export interface AppState {
  platform: Platform;
  /** No stick yet: show the first launch. */
  first_launch: boolean;
}

export function appState(): Promise<AppState> {
  return invoke("app_state");
}

/** Creates the keys and the stick (no size to choose, ADR 0008). */
export function plugIn(): Promise<void> {
  return invoke("plug_in");
}

export interface Entry {
  name: string;
  /** null for a linked entry not read yet. */
  size: number | null;
  is_dir: boolean;
  /** Shared from where it is, not copied (ADR 0008). */
  linked: boolean;
  /** A linked entry that is not there (unplugged, moved). */
  missing: boolean;
  /** In the stick's folder but not shared: a linked entry has its name. */
  left_out: boolean;
}

export interface Sharing {
  state: "off" | "starting" | "on" | "failed";
  message: string | null;
  has_link: boolean;
  readers: number;
  bytes_per_second: number;
  /** What to do when people can download only through the relay. */
  relay_only: string | null;
}

export interface Stick {
  name: string;
  /** Bytes copied onto it. */
  used: number;
  /** What can still be copied onto it: what the computer has room for. */
  left: number;
  entries: Entry[];
  sharing: Sharing;
  /** Mounted (macOS) or in the sidebar (Linux); false after Eject. */
  mounted: boolean;
}

/** A problem with sharing (event `share-problem`). */
export type ShareProblem =
  | { kind: "failed"; message: string }
  | { kind: "ejected" }
  | { kind: "seen_slowly" }
  | { kind: "cleared" };

export interface Activity {
  readers: number;
  bytes_per_second: number;
  /** What to do when people can download only through the relay. */
  relay_only: string | null;
}

export function stick(): Promise<Stick> {
  return invoke("stick");
}

/** What became of a drop. */
export type AddOutcome =
  | { kind: "copied" }
  /** Refused, as by a real stick; nothing was copied. */
  | { kind: "does_not_fit"; needed: number; left: number };

/** Copies dropped files and folders onto the stick, if they fit. */
export function addPaths(paths: string[]): Promise<AddOutcome> {
  return invoke("add_paths", { paths });
}

/** "Share it as it is": links them, shared from where they are. */
export function linkPaths(paths: string[]): Promise<void> {
  return invoke("link_paths", { paths });
}

/** Removes a top-level entry: the stick's copy, or only the link. */
export function removeEntry(name: string): Promise<void> {
  return invoke("remove_entry", { name });
}

export interface ShareView {
  link: string;
  /** The link as a QR code, an SVG made by the app. */
  qr_svg: string;
}

export function openShare(): Promise<void> {
  return invoke("open_share");
}

/** The link and its QR code; null while the stick is not published yet. */
export function shareView(): Promise<ShareView | null> {
  return invoke("share_view");
}

export function copyLink(): Promise<void> {
  return invoke("copy_link");
}

export function copyQrImage(): Promise<void> {
  return invoke("copy_qr_image");
}

export function sendByEmail(): Promise<void> {
  return invoke("send_by_email");
}

export interface ConfirmView {
  sharing: boolean;
  readers: number;
}

export function confirmView(): Promise<ConfirmView> {
  return invoke("confirm_view");
}

/** Shows the Stop sharing dialog, or the Quit one. */
export function openDialog(quit: boolean): Promise<void> {
  return invoke("open_dialog", { quit });
}

export function stopSharing(): Promise<void> {
  return invoke("stop_sharing");
}

export function startSharingAgain(): Promise<void> {
  return invoke("start_sharing_again");
}

export function quit(): Promise<void> {
  return invoke("quit");
}

/** Ejects the stick; sharing goes on. */
export function eject(): Promise<void> {
  return invoke("eject");
}

/** Mounts the stick again. */
export function mount(): Promise<void> {
  return invoke("mount");
}

export function openInFileManager(): Promise<void> {
  return invoke("open_in_file_manager");
}
