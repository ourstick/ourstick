// The only way the UI talks to the app: Tauri commands, which call only
// ourstick-core. Types mirror the Rust side (src-tauri/src/commands.rs).
import { invoke } from "@tauri-apps/api/core";

export type Platform = "linux" | "macos" | "windows" | "android" | "ios";

/** A phone's app (ADR 0027): from the platform the window was given at start. */
export function onPhone(): boolean {
  const platform = document.documentElement.dataset.platform;
  return platform === "android" || platform === "ios";
}

export interface AppState {
  platform: Platform;
  /** macOS: the Local Network step comes first (ADR 0020). */
  explain_local_network: boolean;
  /** macOS: the keys' move comes first, with how many keys (ADR 0021). */
  move_keys: number | null;
}

export function appState(): Promise<AppState> {
  return invoke("app_state");
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
  /** Where a linked entry is: the name of the folder it is in. */
  place: string | null;
  /** The one folder the stick was made from, its contents at the top:
   * never taken out, the stick is stopped as a whole. */
  at_top: boolean;
}

export interface Sharing {
  state: "off" | "starting" | "on" | "failed";
  message: string | null;
  has_link: boolean;
  readers: number;
  bytes_per_second: number;
  preparing: boolean;
  /** What to do when people can download only through the relay. */
  relay_only: string | null;
  /** Sending slowly, so another program can use the disk. */
  giving_way: boolean;
  /** A reader on this network is kept on the relay by macOS (ADR 0020). */
  local_network_declined: boolean;
}

export interface Stick {
  name: string;
  entries: Entry[];
  sharing: Sharing;
  /** Made before ADR 0028: a folder of its own, sent too. */
  own_folder: boolean;
  /** Its own folder is there (not an ejected image, an unplugged disk). */
  folder_there: boolean;
  /** Where an older stick's own files stay if it is removed. */
  folder: string | null;
  /** Its last problem, kept by the app (event `problems-changed`). */
  problem: ShareProblem | null;
  /** When it was last looked at in this run, in seconds since 1970. */
  looked_at: number | null;
}

/** An event about one stick (`activity`). */
export type About<T> = T & { stick_id: string };

/** A problem, of a stick or of the whole app. */
export type ShareProblem =
  | { kind: "failed"; message: string }
  | { kind: "ejected" }
  | { kind: "seen_slowly" };

/** The whole app's problem (not of one stick), if any. */
export function appProblem(): Promise<ShareProblem | null> {
  return invoke("app_problem");
}

/** The user closed the app's problem. */
export function dismissAppProblem(): Promise<void> {
  return invoke("dismiss_app_problem");
}

export interface Activity {
  readers: number;
  bytes_per_second: number;
  /** Readers wait while their files are got ready: nothing leaves. */
  preparing: boolean;
  /** What to do when people can download only through the relay. */
  relay_only: string | null;
  /** Sending slowly, so another program can use the disk. */
  giving_way: boolean;
  /** A reader on this network is kept on the relay by macOS (ADR 0020). */
  local_network_declined: boolean;
}

export function stick(stickId: string): Promise<Stick> {
  return invoke("stick", { stickId });
}

/** `+ Add more`: links them, sent from where they are. */
export function linkPaths(stickId: string, paths: string[]): Promise<void> {
  return invoke("link_paths", { stickId, paths });
}

/** "Find it…": asks where a linked entry that is not found is now, and
 * sends it from there under its name (ADR 0029). False if cancelled. */
export function findEntry(stickId: string, name: string): Promise<boolean> {
  return invoke("find_entry", { stickId, name });
}

/** "Stop sending “Summer”?": takes a linked entry out; its files stay. */
export function removeEntry(stickId: string, name: string): Promise<void> {
  return invoke("remove_entry", { stickId, name });
}

export interface ShareView {
  link: string;
  /** The link as a QR code, an SVG made by the app. */
  qr_svg: string;
}


/** The link and its QR code; null while the stick is not published yet. */
export function shareView(stickId: string): Promise<ShareView | null> {
  return invoke("share_view", { stickId });
}

export function copyLink(stickId: string): Promise<void> {
  return invoke("copy_link", { stickId });
}

/** Whether sending goes on once OurStick is not in front (ADR 0029). */
export function sendsInBackground(): Promise<boolean> {
  return invoke("sends_in_background");
}

/** A phone's "Send the link": its share sheet (ADR 0029). */
export function shareLink(stickId: string): Promise<void> {
  return invoke("share_link", { stickId });
}

export function copyQrImage(stickId: string): Promise<void> {
  return invoke("copy_qr_image", { stickId });
}

export interface ConfirmView {
  sharing: boolean;
  readers: number;
  /** The stick's name (Stop sharing). */
  name: string | null;
  /** Quit: the sticks being shared, and those someone downloads from now. */
  shared: string[];
  downloading: string[];
  /** Quit: received sticks coming in now (they go on next time). */
  receiving: string[];
  /** Quit: received sticks are kept up to date, or waited for. */
  following: boolean;
}

/** About one stick, or (null, for Quit) about every stick. */
export function confirmView(stickId: string | null): Promise<ConfirmView> {
  return invoke("confirm_view", { stickId });
}

/** Shows the Stop sharing dialog, or the Quit one. */
export function openDialog(
  quit: boolean,
  stickId: string | null,
): Promise<void> {
  return invoke("open_dialog", { quit, stickId });
}

/** `Minimize instead`, the Quit dialog's safe choice: the window put away. */
export function minimizeMain(): Promise<void> {
  return invoke("minimize_main");
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

/** "Change the sharing link": the old one stops working. Returns the new id. */
export function newLink(stickId: string): Promise<string> {
  return invoke("new_link", { stickId });
}

/** "Remove from OurStick": its link and keys go; no file of the user's. */
export function removeStick(stickId: string): Promise<void> {
  return invoke("remove", { stickId });
}

/** "Rename…": people see the new name (an older stick's folder follows). */
export function renameStick(stickId: string, name: string): Promise<void> {
  return invoke("rename", { stickId, name });
}

/** A name saved under another here than at the sender (escaped already). */
export interface SavedAs {
  path: string;
  saved_as: string;
  /** Files it holds: 1 for a file, more for a folder. */
  files: number;
}

/** The names saved under another: the first lines, how many more, files in all. */
export interface SavedAsList {
  lines: SavedAs[];
  more: number;
  files: number;
}

/** A stick this device receives in the window (step 7b). */
export interface ReceivedEntry {
  id: string;
  /** Its name from the sender (show it isolated: it is the sender's text). */
  name: string;
  /** Null while its sender is waited for: no folder yet. */
  folder: string | null;
  paused: boolean;
  /** A link waited for (its sender was away): Forget only. */
  waiting: boolean;
  /** What is saved under another name than at the sender. */
  saved_as: SavedAsList;
  /** "starting", "downloading", "getting_more", "up_to_date", "waiting",
   * "paused", "stopped"; a phone's (ADR 0027): "queued", "checked"; a link
   * waited for: "waiting_sender", "wait_ended". */
  state: string;
  /** Why it waits or stopped, in words for everyone. */
  message: string | null;
  done: number;
  total: number;
  /** When it began waiting, or was checked (ms since 1970), for "since 14:30". */
  since_ms: number | null;
  /** The sender is on this network, but macOS keeps OurStick from it (ADR 0020). */
  local_network_declined: boolean;
  /** What is not got (its last part, escaped), whether its files are
   * already here, and the folder it is inside (escaped; null at the top). */
  left_out: { shown: string; here: boolean; inside: string | null }[];
}

export function received(): Promise<ReceivedEntry[]> {
  return invoke("received");
}

export * from "./previewApi";
export * from "./choiceApi";
export * from "./readersApi";

export function pauseReceived(stickId: string): Promise<void> {
  return invoke("pause_received", { stickId });
}

export function resumeReceived(stickId: string): Promise<void> {
  return invoke("resume_received", { stickId });
}

/** "Forget this stick": no longer kept up to date; its files stay. */
export function forgetReceived(stickId: string): Promise<void> {
  return invoke("forget_received_stick", { stickId });
}

/** "Move to another folder…": what came moves with it into `into`. */
export function moveReceived(stickId: string, into: string): Promise<void> {
  return invoke("move_received", { stickId, into });
}

export function openReceivedFolder(stickId: string): Promise<void> {
  return invoke("open_received_folder", { stickId });
}
