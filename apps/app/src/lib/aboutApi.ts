// About and start at login (ADR 0024). Types mirror src-tauri/src/about.rs
// and src-tauri/src/login/commands.rs.
import { invoke } from "@tauri-apps/api/core";

export interface AboutView {
  version: string;
  /** The bundles' copyright line. */
  copyright: string;
}

export function aboutView(): Promise<AboutView> {
  return invoke("about_view");
}

export function openAbout(): Promise<void> {
  return invoke("open_about");
}

/** A link of About, by name: the app opens its fixed target. */
export type AboutLink = "website" | "privacy" | "licence" | "libraries" | "report";

export function openAboutLink(link: AboutLink): Promise<void> {
  return invoke("open_about_link", { link });
}

export interface StartAtLogin {
  /** What the check shows: off if the system turned it off. */
  on: boolean;
  /** The window said once that the app starts at login. */
  told: boolean;
}

export function startAtLogin(): Promise<StartAtLogin> {
  return invoke("start_at_login");
}

export function setStartAtLogin(on: boolean): Promise<void> {
  return invoke("set_start_at_login", { on });
}

export function startAtLoginTold(): Promise<void> {
  return invoke("start_at_login_told");
}
