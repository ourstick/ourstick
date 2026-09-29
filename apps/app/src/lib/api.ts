// The only way the UI talks to the app: Tauri commands, which call only
// ourstick-core. Types mirror the Rust side (src-tauri/src/commands.rs).
import { invoke } from "@tauri-apps/api/core";

export type Platform = "linux" | "macos" | "windows";

export interface AppState {
  platform: Platform;
  /** No stick yet: show the first launch. */
  first_launch: boolean;
}

export const STICK_SIZES_GB = [5, 10, 20, 50] as const;

export function appState(): Promise<AppState> {
  return invoke("app_state");
}

/** Creates the keys and the stick of `sizeGb` GB. */
export function plugIn(sizeGb: number): Promise<void> {
  return invoke("plug_in", { sizeGb });
}

export interface Entry {
  name: string;
  size: number;
  is_dir: boolean;
}

export interface Sharing {
  state: "off" | "starting" | "on" | "failed";
  message: string | null;
  has_link: boolean;
  readers: number;
  bytes_per_second: number;
}

export interface Stick {
  name: string;
  size: number;
  used: number;
  entries: Entry[];
  sharing: Sharing;
}

/** A problem with sharing (event `share-problem`). */
export type ShareProblem =
  | { kind: "failed"; message: string }
  | { kind: "ejected" }
  | { kind: "cleared" };

export interface Activity {
  readers: number;
  bytes_per_second: number;
}

export function stick(): Promise<Stick> {
  return invoke("stick");
}

/** Copies dropped files and folders onto the stick. */
export function addPaths(paths: string[]): Promise<void> {
  return invoke("add_paths", { paths });
}

/** Removes a file or folder from the stick (the stick's copy). */
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
