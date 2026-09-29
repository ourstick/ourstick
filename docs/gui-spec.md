# OurStick — GUI spec (stage 1: desktop)

Reference image: `gui-mockup.png` (5 panels). This spec plus the image are the
input for the Tauri app. Anything not on the image or in this file is out of
scope for stage 1.

## Principle

The app is a virtual USB stick. The user should never need to learn anything
they don't already know from a physical stick. Every screen has the minimum
number of elements. No settings page in stage 1.

## Platforms

- Linux: full experience, including mount (FUSE).
- Windows, macOS: same window, **no mount**. Files are dropped on the window.
  The "Mounted" pill is replaced by nothing; the menu has no Eject.
- Mobile: not in stage 1.

## Panels

### 1 · Main window

- Title: `OurStick (10 GB)` — the size chosen at first launch.
- Right: pill `● Mounted ▾` (Linux only). Click opens a two-item menu:
  `Open in file manager`, `Eject (keeps sharing)`.
- Drop zone: "Drop files or folders here", below it `N files · size`.
  Dropping copies nothing: it adds the path to the stick (same as CLI share).
- File list: name and size, folders with trailing `/`. Read-only list;
  removing is done in the file manager (Linux) or via right-click → Remove (all).
- Capacity: `used / stick size`, bar. Stick size is the cache budget: when
  exceeded, the drop is refused with "Stick is full (10 GB)".
- Activity line (only while someone downloads):
  `● 1 person downloading · 2.1 MB/s · direct` (or `relay`).
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
- If a download is in progress: bold line `<name or "Someone"> is still
  downloading — X GB remaining.`
- Buttons: `Keep sharing`, `Stop` (red). Quit shows the same dialog.

### 5 · First launch

- Title "Welcome to OurStick", three sentences (see image).
- Stick size dropdown: 5 / 10 / 20 / 50 GB, default 10.
- Button `Plug it in` → creates keys, mounts (Linux), opens panel 1.
- Shown once. Size can't be changed in stage 1 (documented limitation).

## Semantics that must hold

- **Eject ≠ stop sharing.** Eject only unmounts. Sharing continues, tray stays.
- **Stop sharing** is the only thing that stops serving. Always confirmed.
- **Link is the key.** Never logged, never sent anywhere by the app itself.
- **Relay policy (ADR 0006)** stays, but the question becomes a dialog:
  "Connection is slow (no direct path). Continue anyway?" — only on the
  receiver side, which is CLI/mobile for now, so no dialog in stage 1.
- **Sleep:** while someone is downloading, inhibit system sleep
  (org.freedesktop.login1 on Linux; later caffeinate / SetThreadExecutionState).

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

## Out of scope (stage 1)

Windows/macOS mount, right-click "Send to" in file managers, multiple sticks,
permissions, expiry, revoke, settings page, auto-start at login, updates.
