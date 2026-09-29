# OurStick — GUI spec (stage 1: desktop)

Reference image: `gui-mockup.png` (5 panels). This spec plus the image are the
input for the Tauri app. Anything not on the image or in this file is out of
scope for stage 1.

## Principle

The app is a virtual USB stick. The user should never need to learn anything
they don't already know from a physical stick. Every screen has the minimum
number of elements. No settings page in stage 1.

## The stick is a folder

The stick is one ordinary folder, like the contents of a real USB stick. Files
dropped on the window are **copied** into it, as onto a real stick; the
originals are not touched. Sharing, the watcher and the size limit all work on
that folder. The app shows the folder as a drive, per platform:

- **macOS:** a sparse APFS disk image,
  `hdiutil create -type SPARSE -fs APFS -size <size> -volname OurStick`, kept in
  `~/Library/Application Support/OurStick/` and attached with `hdiutil attach`.
  It appears in Finder's Locations as a drive with an eject button. The mounted
  volume is the stick folder. Ejecting in Finder is a real detach, and the app
  notices. The image enforces the size limit.
- **Linux:** the folder `~/OurStick`, added as a bookmark to the file manager's
  sidebar (GTK bookmarks for Nautilus, `user-places.xbel` for Dolphin) with a
  USB drive icon where the file manager allows one. No loop device (it needs
  root). The watcher enforces the size limit.
- **Windows:** not in stage 1 (later: a VHDX with `Mount-DiskImage`, or `subst`
  as a simple alternative).
- **Mobile:** not in stage 1.

**Eject** detaches the image (macOS) or removes the bookmark (Linux). Sharing
goes on with the last published version; changes are picked up again after
Mount. An ejected stick is never published as empty.

Hidden files that operating systems and file managers put on drives are never
part of the stick (see "Owner-side rules" in `docs/protocol.md`): `.DS_Store`,
`._*`, `.Spotlight-V100`, `.fseventsd`, `.Trashes`, `.TemporaryItems`,
`.Trash-*`, `.directory`, and the like. The image gets `.metadata_never_index`
so Spotlight leaves it alone.

## Panels

### 1 · Main window

- Title: `OurStick (10 GB)` — the size chosen at first launch.
- Right: pill `● Mounted ▾` (macOS, Linux; `○ Ejected ▾` after eject). Click
  opens a two-item menu: `Open in file manager`, `Eject (keeps sharing)` (or
  `Mount` when ejected).
- Drop zone: "Drop files or folders here", below it `N files · size`.
  Dropping **copies** into the stick folder, like onto a real stick. A drop
  that does not fit is refused with "Stick is full (10 GB)".
- File list: the top level of the stick folder, name and size, folders with
  trailing `/`. Removing is done in the file manager, or via right-click →
  Remove, which deletes the copy on the stick (never an original).
- Capacity: `used / stick size`, bar. If the folder grows past the size anyway
  (on Linux, e.g. by copying in the file manager), the stick is not published
  and the window shows "Stick is full, remove N MB".
- Activity line (only while someone downloads):
  `● Someone downloading · 2.1 MB/s`, or `● 2 downloading · 4.3 MB/s`.
  Readers have no names, and the owner cannot tell how much a reader still
  needs (see "Open questions"). `· direct` / `· relay` is added only if iroh
  gives it cheaply on the owner's side.
- `⤴ Share` button → panel 2.

### 2 · Share popup

- One line: "Anyone with this link can read the stick."
- Link field (read-only, monospace) + `Copy`.
- QR code of the full link (including `#secret`).
- `Copy QR image`, `Send by email` (opens mailto: with the link in the body).
- Nothing else. No expiry, no permissions, no revoke in stage 1.

### 3 · Tray icon

- Always present while the app runs. Green dot when someone is downloading.
- Menu: `Open OurStick`, `Share…`, `Eject / Mount` (Linux), `Stop sharing…`
  (red), `Quit`.
- Closing the main window minimizes to tray; it does not stop sharing.

### 4 · Stop sharing (also on Quit)

- "Stop sharing OurStick?" / "Anyone with the link will no longer be able to
  download."
- If a download is in progress: bold line `Someone is still downloading.`
  (no names, no remaining size in stage 1).
- Buttons: `Keep sharing`, `Stop` (red). Quit shows the same dialog.

### 5 · First launch

- Title "Welcome to OurStick", three sentences (see image).
- Stick size dropdown: 5 / 10 / 20 / 50 GB, default 10.
- Button `Plug it in` → creates keys (Secret Service on Linux, Keychain on
  macOS), creates the stick folder (the disk image on macOS, `~/OurStick` on
  Linux), mounts it or adds the bookmark, and opens panel 1.
- Shown once. Size can't be changed in stage 1 (documented limitation).

## Semantics that must hold

- **Eject ≠ stop sharing.** Eject only unmounts. Sharing continues, tray stays.
- **Stop sharing** is the only thing that stops serving. Always confirmed.
- **Link is the key.** Never logged, never sent anywhere by the app itself.
- **Relay policy (ADR 0006 v2):** never stop, never ask. Without a direct
  path, data goes through the relay at a limited speed, with a message that
  stays visible. That is on the receiver side (CLI for now); nothing to show
  in the stage 1 window.
- **Sleep:** while someone is downloading, inhibit system sleep
  (`org.freedesktop.login1` on Linux, a power assertion on macOS).

## Known limitations (stage 1)

- The stick size cannot be changed after first launch.
- The encrypted cache is a second copy of the stick: a 10 GB stick can take up
  to about 20 GB of disk.
- macOS: a sparse image does not shrink when files are deleted; it gets
  smaller only after `hdiutil compact` while detached.
- Linux: the bookmark's icon is up to the file manager; Nautilus shows its
  bookmarks with a folder icon.

## Theming: native look per platform

Goal: the window should feel like a system dialog of the host OS (a drive
properties or "safely remove" dialog), not like a web app. The structure is
identical everywhere; only a CSS theme changes. Detect the platform once at
startup (Tauri `os` plugin) and set `data-platform` on `<html>`.

Do not chase pixel-perfect imitation of Finder / Explorer / Nautilus. Aim for
"does not look foreign". Use system fonts, system accent color where readable,
and the platform's spacing and corner radius.

| | Linux (GNOME default) | Windows 11 | macOS |
|---|---|---|---|
| Font | `Cantarell`, fallback `system-ui` | `Segoe UI Variable`, `Segoe UI` | `-apple-system`, `SF Pro` |
| Base size | 14px | 14px | 13px |
| Window bg | `#fafafa` light / `#242424` dark | `#f3f3f3` / `#202020` (Mica look) | `#ececec` / `#1e1e1e` |
| Card / list bg | `#ffffff` / `#303030` | `#ffffff` / `#2b2b2b` | `#ffffff` / `#282828` |
| Border | `#d8d8d8` / `#3a3a3a` | `#e5e5e5` / `#383838` | `#d1d1d1` / `#3c3c3c` |
| Corner radius | 8px (buttons 6px) | 4px (Fluent) | 10px (buttons 6px) |
| Accent | `#3584e4` (Adwaita blue) | system accent via `AccentColor` CSS, fallback `#0067c0` | `#0a84ff`, or `AccentColor` |
| Primary button | filled accent | filled accent | filled accent, 22px height |
| Danger | `#e01b24` | `#c42b1c` | `#ff3b30` |
| Title bar | app draws none; use CSD with GTK-like header (Tauri `decorations: true`) | native | native, `titleBarStyle: overlay` |
| Dark mode | follow `prefers-color-scheme` | same | same |

Rules:
- Use `AccentColor` / `AccentColorText` CSS system colors where the WebView
  supports them; fall back to the table.
- Icons: symbolic/monochrome line icons (Adwaita-style on Linux, Fluent-style
  on Windows, SF-Symbols-like on macOS). One icon set with per-platform
  stroke weight is enough; do not ship three sets.
- Tray icon: monochrome, follows the panel theme (light/dark), green dot for
  activity.
- KDE, XFCE etc. get the GNOME theme; refine later if users ask.
- Never use a non-system font. No web fonts, no downloads.

## Receiver side

Not in stage 1. Receivers use `ourstick get` (CLI) until the receiver window
and mobile apps exist. The link page at ourstick.com/s/ says how to install.

## Open questions

- **Link a folder without copying** it onto the stick (share in place, as the
  CLI does).
- **"X GB remaining"** for the owner: needs a small addition to the protocol,
  where the reader tells the owner how much it still needs.
- **macOS mount point:** whether Finder shows the volume in Locations when it
  is attached elsewhere than `/Volumes` (to be tried on the MacBook). Under
  `/Volumes` its name can change (`OurStick 1`), so the app identifies the
  stick by its image, not by the path.
- **Windows:** VHDX with `Mount-DiskImage`, or `subst`.

## Out of scope (stage 1)

Windows (build and drive), FUSE or any virtual filesystem, right-click "Send
to" in file managers, multiple sticks, permissions, expiry, revoke, settings
page, auto-start at login, updates, the receiver window.
