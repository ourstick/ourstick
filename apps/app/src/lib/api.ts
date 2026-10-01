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

/** Creates the keys and the first stick, called `name` (no size to choose, ADR 0008). */
export function plugIn(name: string): Promise<void> {
  return invoke("plug_in", { name });
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

/** An event about one stick; none for a problem of the whole app. */
export type About<T> = T & { stick_id?: string };

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

export function stick(stickId: string): Promise<Stick> {
  return invoke("stick", { stickId });
}

/** What became of a drop. */
export type AddOutcome =
  | { kind: "copied" }
  /** Refused, as by a real stick; nothing was copied. */
  | { kind: "does_not_fit"; needed: number; left: number };

/** Copies dropped files and folders onto the stick, if they fit. */
export function addPaths(stickId: string, paths: string[]): Promise<AddOutcome> {
  return invoke("add_paths", { stickId, paths });
}

/** "Share it as it is": links them, shared from where they are. */
export function linkPaths(stickId: string, paths: string[]): Promise<void> {
  return invoke("link_paths", { stickId, paths });
}

/** Removes a top-level entry: the stick's copy, or only the link. */
export function removeEntry(stickId: string, name: string): Promise<void> {
  return invoke("remove_entry", { stickId, name });
}

export interface ShareView {
  link: string;
  /** The link as a QR code, an SVG made by the app. */
  qr_svg: string;
}

export function openShare(stickId: string): Promise<void> {
  return invoke("open_share", { stickId });
}

/** The link and its QR code; null while the stick is not published yet. */
export function shareView(stickId: string): Promise<ShareView | null> {
  return invoke("share_view", { stickId });
}

export function copyLink(stickId: string): Promise<void> {
  return invoke("copy_link", { stickId });
}

export function copyQrImage(stickId: string): Promise<void> {
  return invoke("copy_qr_image", { stickId });
}

export function sendByEmail(stickId: string): Promise<void> {
  return invoke("send_by_email", { stickId });
}

export interface ConfirmView {
  sharing: boolean;
  readers: number;
  /** The stick's name (Stop sharing). */
  name: string | null;
  /** Quit: the sticks being shared, and those someone downloads from now. */
  shared: string[];
  downloading: string[];
}

/** About one stick, or (null, for Quit) about every stick. */
export function confirmView(stickId: string | null): Promise<ConfirmView> {
  return invoke("confirm_view", { stickId });
}

/** Shows the Stop sharing dialog, or the Quit one. */
export function openDialog(quit: boolean, stickId: string | null): Promise<void> {
  return invoke("open_dialog", { quit, stickId });
}

export function stopSharing(stickId: string): Promise<void> {
  return invoke("stop_sharing", { stickId });
}

export function startSharingAgain(stickId: string): Promise<void> {
  return invoke("start_sharing_again", { stickId });
}

export function quit(): Promise<void> {
  return invoke("quit");
}

/** Ejects the stick; sharing goes on. */
export function eject(stickId: string): Promise<void> {
  return invoke("eject", { stickId });
}

/** Mounts the stick again. */
export function mount(stickId: string): Promise<void> {
  return invoke("mount", { stickId });
}

export function openInFileManager(stickId: string): Promise<void> {
  return invoke("open_in_file_manager", { stickId });
}

/** One of the app's sticks, as the list shows it. */
export interface StickEntry {
  /** Its id (hex), which every stick command takes. */
  id: string;
  name: string;
  sharing: "off" | "starting" | "on" | "failed" | "busy";
  /** While busy, what is being done ("Renaming…"). */
  doing: string | null;
  readers: number;
}

/** The app's sticks, oldest first. */
export function sticks(): Promise<StickEntry[]> {
  return invoke("sticks");
}

/** "Give it a new link": everyone with the old link loses it. Returns the new id. */
export function newLink(stickId: string): Promise<string> {
  return invoke("new_link", { stickId });
}

/** "Remove this stick": its copied files go, with its link. */
export function removeStick(stickId: string): Promise<void> {
  return invoke("remove", { stickId });
}

/** "New stick…": a stick of its own, with its own link. Returns its id. */
export function newStick(name: string): Promise<string> {
  return invoke("new_stick", { name });
}

/** "Rename…": its folder and drive follow; people see the new name. */
export function renameStick(stickId: string, name: string): Promise<void> {
  return invoke("rename", { stickId, name });
}
