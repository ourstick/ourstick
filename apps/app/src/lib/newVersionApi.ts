// A new version (ADR 0033). Types mirror src-tauri/src/new_version.rs.
import { invoke } from "@tauri-apps/api/core";

export interface NoticeLine {
  kind: "newer" | "security" | "key-changed" | "old";
  /** "OurStick 0.3.0 is out". */
  text: string;
  /** "Get it", or "ourstick.com". */
  action: string;
}

export interface NoticeView {
  line: NoticeLine | null;
  /** About shows the setting: a desktop build that reads the notice. */
  setting: boolean;
  looking: boolean;
  lastGoodMs: number | null;
  failingSinceMs: number | null;
  failure: "no-connection" | "refused" | "not-the-file" | "old" | null;
  /** Told once: the app now looks. */
  tellLooking: boolean;
  /** Told once: the security line. */
  tellSecurity: boolean;
}

export function versionNotice(): Promise<NoticeView> {
  return invoke("version_notice");
}

export function setLookForVersions(on: boolean): Promise<void> {
  return invoke("set_look_for_versions", { on });
}

export function versionTold(what: "looking" | "security"): Promise<void> {
  return invoke("version_told", { what });
}

/** The app's fixed download page: never a URL from the notice. */
export function openDownloadPage(): Promise<void> {
  return invoke("open_download_page");
}
