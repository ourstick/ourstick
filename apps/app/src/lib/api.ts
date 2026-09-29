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
