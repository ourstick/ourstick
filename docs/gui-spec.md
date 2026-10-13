# OurStick — GUI spec (stage 1: desktop)

Reference image: `gui-mockup.png` (5 panels), the first mockup, kept as
history: it shows one stick of a fixed size (ADR 0008 removed sizes) and a
receiver by name (ADR 0015 never names a computer). Where they differ, this
file and the decisions are right. Anything not in this file is out of scope
for stage 1.

## Principle

The app is a virtual USB stick. The user should never need to learn anything
they don't already know from a physical stick. Every screen has the minimum
number of elements. No settings page in stage 1.

## The stick is a folder

The stick is one ordinary folder, like the contents of a real USB stick. Files
dropped on the window are **copied** into it, as onto a real stick; the
originals are not touched. Sharing and the watcher work on that folder. The
stick has **no fixed size** (ADR 0008): what can still be copied onto it is
what the computer has room for. The app shows the folder as a drive, per
platform:

- **macOS:** a sparse APFS disk image,
  `hdiutil create -type SPARSE -fs APFS -size <its disk's size> -volname OurStick`, kept in
  `~/Library/Application Support/OurStick/` and attached with `hdiutil attach`.
  It appears in Finder's Locations as a drive with an eject button. The mounted
  volume is the stick folder. Ejecting in Finder is a real detach, and the app
  notices. Sparse, it takes room only as it fills, so it never limits first;
  an image made with a size before is grown at start: it stays attached
  after Quit, so the app detaches it, grows it and attaches it again (it
  disappears from Finder for a moment); if a file on it is open, it keeps its
  size until the next start. Tested on a 5 GB image: grown to the disk's
  245 GB, its files kept, 17 MB on disk. What is left is
  measured on the disk that holds the image, not on the mounted volume.
- **Linux:** the folder `~/OurStick`, added as a bookmark to the file manager's
  sidebar (GTK bookmarks for Nautilus, `user-places.xbel` for Dolphin) with a
  USB drive icon where the file manager allows one. No loop device (it needs
  root).
- **Windows:** the folder `%USERPROFILE%\OurStick`, shown as a drive letter
  with `subst` (no admin rights): the letter it had before (kept in the
  database), else `O:`, else `Z:` down to `D:`. A `subst` letter lasts until
  logoff, so the app gives it again at every start. What is left is measured
  on the folder's disk, not by the letter. The letter carries the system
  disk's name ("Windows (O:)"), so the stick is also an entry at the root of
  File Explorer's navigation pane, under its own name and OurStick's icon
  (ADR 0023).
- **Mobile:** not in stage 1.

**Eject** detaches the image (macOS), removes the bookmark (Linux) or takes
the drive letter and the pane entry away (Windows). Sharing
goes on with the last published version; changes are picked up again after
Mount. An ejected stick is never published as empty.

Hidden files that operating systems and file managers put on drives are never
part of the stick (see "Owner-side rules" in `docs/protocol.md`): `.DS_Store`,
`._*`, `.Spotlight-V100`, `.fseventsd`, `.Trashes`, `.TemporaryItems`,
`.Trash-*`, `.directory`, and the like. The image gets `.metadata_never_index`
so Spotlight leaves it alone.

## Panels

### 1 · Main window

- Left: the app's sticks (ADR 0009), oldest first, each with its name and
  what it is doing in plain words: `Sharing`, `Someone downloading`,
  `2 downloading`, `Not sharing`, `Getting ready…`, or while it changes
  `Renaming…`, `Changing the sharing link…`, `Removing…`. One is shown on the
  right; at the bottom `+ New stick…` asks for a name (`Make it`; "Making
  it… please wait."). The tray's `New stick…` opens the same field.
- Right, the chosen stick. Title: its name; no size.
- Next to it, pill `● Mounted ▾` (macOS, Linux; `○ Ejected ▾` after eject).
  Click opens a two-item menu: `Open in file manager`, `Eject (keeps
  sharing)` (or `Mount` when ejected).
- And `⋯`, the stick's own menu: `Rename…`, `Stop sharing…` (or `Start
  sharing`), `Change the sharing link…`, `Remove this stick…` (red). Each asks
  in the window first, with "… please wait." while it works:
  - Rename: the new name (1–60 characters, not the name of another stick).
    Its folder or volume, its bookmark or letter follow; people see the new
    name with its next version.
  - Change the sharing link (the only way to take access back, ADR 0009
    §5): "The old sharing link stops working. Send the new one to the people
    who should still have it. The files stay." Button `Change it`. "Link" is
    always "sharing link" in the window, so that nobody wonders which link.
  - Remove: "Its sharing link stops working, and the files copied onto it are
    deleted from this computer. Folders shared as they are (linked) stay
    where they are."
- Drop zone: "Drop files or folders here, or click to choose", below it
  what is on the stick, `3 folders · 1 file · 6.9 GB`. A click (or Enter:
  it is a button) opens `Add files…` / `Add folders…`, the system's picker
  for several at once (Linux's and Windows' pickers do not take files and
  folders together); what is chosen is added as a drop is, the same queue
  and questions. Dropping **copies** into the stick folder,
  like onto a real stick. A drop that does not fit in what is left is
  refused, as by a real stick: nothing is copied, and the window says how
  big it is and how much is left. While it copies, the zone says `Copying…
  1.2 GB of 3.1 GB` with a bar. A drop meanwhile waits its turn: one copy
  at a time, the next once the one before is done, in the order dropped;
  under the bar, in grey, "Next: Wallpaper, 2021-08" with `Don't add
  these` (the ones not started). A drop that asks something ("already on
  the stick", "does not fit") holds the queue until it is answered or
  cancelled. What fails is said in red with the drop it was ("Wallpaper:
  …"), each kept while the queue goes on. The queue is the stick's:
  another stick's view and back finds it as it was; Quit drops what has
  not started. While a copy goes on, the stick is not renamed, moved,
  given a new link or removed ("a copy onto the stick is still going on;
  try again once it is done"), and while it is being changed a drop is
  refused ("The stick is being changed …; drop again once that is
  done."). A name already on the stick is asked about
  before anything is copied: "“new in” is already on the stick." (or "“new
  in” and 1 more is…", "… and 2 more are…"), "Replace changes only the
  stick's copy, never the original.", `Cancel` (also Escape), `Keep both`
  (the new one as "new in (2)"), `Replace`. The copy is made under a hidden
  name and takes its place only once whole: a failed copy leaves the old
  one. A linked entry's name is not asked about: it is never replaced, the
  new one goes beside it. Something already on the stick (also through a
  link or the stick's drive letter) is refused: "it is already on the
  stick".
- A drop that does not fit offers **Share it as it is** (ADR 0008 §2): the
  folders or files are linked, shared from where they are, not copied, taking
  no room on the stick and with no size limit of their own. The message says
  that moving or deleting them there takes them off the stick too. A path
  that is, holds or lies inside the stick, the data folder or another linked
  entry is refused, and says why. While it is added (checking and reading the
  folder can take a moment) the message says "Adding… please wait." and its
  buttons wait.
- File list: the top level of the stick, name and size, folders with
  trailing `/`: the stick folder's entries and the linked ones, marked
  `linked` (their size is what the last read found, "reading…" before). A
  linked entry that is not there (unplugged, moved) is marked `not available`;
  people still get its last version. An entry of the stick folder with a
  linked entry's name (ignoring case) is marked `hidden: same name`. Words
  for everyone: no other jargon than "linked". Removing is
  done in the file manager, or via right-click → Remove, which deletes the
  copy on the stick (never an original); for a linked entry it is **Stop
  sharing**, which removes only the link, never its files.
- If some folders cannot be watched (e.g. the system's limit on watches), a
  line says their changes are seen within a minute.
- While the stick's folder is read and another program uses the same disk,
  a line says "Reading your folder slowly, so another program can use the
  disk." (ADR 0013), until OurStick reads at full speed again. It is a note,
  not a problem: the event `giving-way`, told at each change.
- Capacity: `2.1 GB used · 128 GB left`, like a real stick, and a bar of
  used out of used + left. "Left" is what the computer has room for once its
  reserve and every download's claim are counted (the space
  ledger, ADR 0008); several sticks on one disk share it.
- Activity line (only while someone downloads):
  `● Someone downloading · 2.1 MB/s`, or `● 2 downloading · 4.3 MB/s`: the
  rate over the last ten seconds (a big stick can be sent in bursts). While readers wait for their files to
  be got ready (told "not ready", nothing sent since: the owner is making a
  file's hash or outboard, ADR 0017): `● Someone downloading · getting their
  files ready`. While OurStick gives way to another program on a disk the
  files are read from (ADR 0017, amendment 1), a line under it: `Sending
  slowly, so another program can use the disk.`
  `· direct` / `· relay` is added only if iroh gives it cheaply on the
  owner's side.
- Who has it (ADR 0015), under the activity: one line per computer that
  received the stick, in its receiver's own words (what each computer
  says of itself, never a proof), the latest first, at most 5 then "and 12
  more": "A Mac has all of it, up to date.", "A Windows PC has what it
  chose, up to date.", "A Mac has everything so far; you are still reading
  your folder." (or "what it chose so far"), "A Linux PC had all of it at
  14:30; not what changed since." (or "on 3 Oct" for another day; "what it
  chose"), "A Mac is downloading: 60%.", "A Mac is waiting: no room
  there." / "…: its folder is not there." / "A Mac was waiting for this
  computer." / "A Mac is waiting.", "A Mac paused it." A computer is named
  by its kind (Mac, Windows PC, Linux PC, computer); only if its receiver
  typed a name is it shown, "Maria (Mac)"; two of a kind with no name are
  told apart by when first heard, "A Mac (since 3 Oct)". When every line
  (shown or not) is up to date: "Everyone who received it is up to
  date."; with none, nothing at all (an empty list explains nothing).
  Each line has `×` (Remove
  from this list); `Clear this list` and "and 12 more" under them, then
  "What each computer says of itself." Never "safe to delete": what was
  received stays with the receivers.
- `⤴ Share` button → panel 2.

### 2 · Share popup

- One line: "Anyone with this sharing link can read the stick."
- Link field (read-only, monospace) + `Copy`.
- No `Send by email`: most mail is read in a browser, where a mailto:
  link opens nothing or a mail program never set up. Copy, and paste
  where one writes.
- No QR code until there is a phone app: a phone cannot open a link yet,
  and a QR image passed on by mail cannot be clicked. The code that makes
  it stays (`qr.rs`, `copy_qr_image`; `SHOW_QR` in `Share.svelte`): with a
  phone app, the QR of the full link (including `#secret`) and `Copy QR
  image` come back.
- Nothing else. No expiry, no permissions, no revoke in stage 1.

### 3 · Tray icon

- Always present while the app runs. Green dot when someone is downloading
  from any stick.
- Menu: `Open OurStick`; then each stick, `Name — sharing` or `Name — not
  sharing`, with its own submenu: `Share…`, `Eject` / `Mount`, `Stop
  sharing…` / `Start sharing`; then `New stick…`; then `About OurStick`,
  the `Start at login` check and `Quit`. The menu is made again whenever a
  stick is made, renamed, removed, mounted, ejected, or starts or stops
  sharing, and when the check changes.
- The tray is a help, never the only way: many never see it (on Windows it
  is often under `^`; on Ubuntu a friend took the Dock's icon for it). The
  main window's ✕, on every system, shows the Quit dialog with `Minimize`
  instead of `Don't quit`: Minimize leaves the window in the taskbar or the
  Dock, where it is seen and found again; it is never hidden. With nothing
  going on (no stick shared or coming in, no drop being copied), ✕ quits
  without asking. Other windows (Share, About, dialogs) just close.
- The Quit dialog (from ✕, the tray, the Dock or ⌘Q) also has the start at
  login check, as in About: the moment one thinks of it. It changes the
  choice at once, whichever button follows.
- Linux: the tray is a StatusNotifierItem of the app's own (`ksni`), not
  Tauri's, because it has to receive the shell's XDG activation token (see
  Known limitations). GNOME shows it through the AppIndicator extension,
  which Ubuntu ships; without such a host there is no tray icon, and the app
  logs why. At login the host may come later: the app tries for 30 seconds,
  then, if it was started at login, shows its window rather than run unseen.

### Start at login, and About (ADR 0024)

- The app starts at login, on by default: the system's own entry (Windows
  `Run`, a macOS LaunchAgent, a Linux autostart file), written by the
  installed app only. Started so (`--autostarted`), it opens no window and
  shares its sticks; the window opens anyway for the keys' move, the Local
  Network step or the notice. The notice, once, at the top of the main
  window: "OurStick opens when you log in, so that your sticks are shared
  after a restart. You can turn this off in the tray menu or in About."
- The system's own "off" (Task Manager, Startup Applications) is kept; the
  check shows it where it can be read (Windows, Linux).
- About: the icon, name, version and tagline; ourstick.com, Privacy, Report
  a problem; the start at login check; the copyright line; OurStick's
  licence and the libraries' licences (the installed files). From the
  tray, the version line at the foot of the sidebar, and the macOS app
  menu.
- macOS: a click on the Dock icon shows the window.

### 4 · Stop sharing (also on Quit)

- "Stop sharing “Photos”?" (the stick's name) / "Anyone with the link will
  no longer be able to download."
- If a download is in progress: bold line `Someone is still downloading.`
  (no names, no remaining size in stage 1).
- Buttons: `Keep sharing`, `Stop` (red). Quit shows the same dialog for every
  stick: "Sharing “Photos” and “Films” stops: anyone with their links will
  no longer be able to download.", and in bold the sticks someone downloads
  from now. A drop being copied is named too, in bold: "Copying onto
  “Photos” is not done. It stops, and nothing of it is kept: drop it again
  next time." (Quit is then red.) What it left, hidden and never shared,
  is removed at the next start for each stick plugged in, else at the
  stick's next drop.

### 5 · First launch

- Title "Welcome to OurStick", three sentences (see image).
- "Name of your stick", with `OurStick` filled in.
- Button `Plug it in` → creates keys (Secret Service on Linux, Keychain on
  macOS), creates the stick folder (the disk image on macOS, `~/OurStick` on
  Linux), mounts it or adds the bookmark, and opens panel 1.
- Shown once. There is no size to choose (ADR 0008).

## macOS: the Local Network permission (ADR 0020)

- At the first start of a version with ADR 0020, on macOS only, the window
  opens on one screen before anything else: "macOS will ask you a
  question. Choose Allow, so OurStick can send data directly to nearby
  computers, much faster." `Continue`. Nothing binds an endpoint before it, so macOS asks right
  after; shown once per device.
- While a declined permission keeps the other computer, on this network,
  on the relay: on the received stick, or under a stick's activity, "This
  is slow: the sender (someone downloading) is on your network, but
  OurStick is not allowed to reach it. (System Settings → Privacy &
  Security → Local Network)" and `Open Settings`. It goes once allowed.

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

- Windows: Windows Firewall may ask whether OurStick may use the network (on
  a "Public" network it did not ask; it blocked quietly). Blocking does not
  always stop direct paths: in the Windows VM, with inbound block rules like
  those "Cancel" makes, a reader on another machine still came directly
  (over IPv6), since hole punching starts from inside. When readers do come
  only through the relay, the app notices (`docs/protocol.md`, "Reader
  paths") and says how to allow OurStick in Windows Security; the same
  message appears on Linux and macOS. That message has not yet been seen
  with a real reader: it needs one that cannot come directly.
- macOS: a sparse image does not shrink when files are deleted; it gets
  smaller only after `hdiutil compact` while detached.
- Linux: the bookmark's icon is up to the file manager; Nautilus and Thunar
  show bookmarks with a folder icon (the folder itself gets the USB drive icon
  where the file manager honours `metadata::custom-icon-name`).
- Linux: the app runs WebKitGTK without compositing
  (`WEBKIT_DISABLE_COMPOSITING_MODE=1`, unless the user set it). With it,
  WebKitGTK draws a scrolling list on its own layer with other antialiasing,
  and the file names turned bold once the list scrolled. The UI is the same
  everywhere; this is only how the Linux engine draws it.
- Linux (GNOME on Wayland): an app may bring its own window forward only with
  an XDG activation token from the user's click; without one GNOME shows
  "OurStick is ready" instead. The shell passes the token of a click on the
  tray to the tray item (`ProvideXdgActivationToken`), which Tauri's tray and
  released `ksni` do not take: `third_party/ksni` is `ksni` 0.3.6 with that
  method, pending upstream (https://github.com/iovxw/ksni/pull/43). The app
  hands the token to GTK and presents the window with it. Three consequences:
  - The main window is minimized, never hidden. GTK 3 makes
    a hidden window anew when it is shown, and a new window comes too late
    for the token.
  - The token raises only a window GNOME already has. So a tray item that
    opens a new window (Share…, Stop sharing…, Quit) first brings the main
    window forward with the token, and opens the new window once the app has
    the focus.
  - Started from a terminal (no launcher, so no token), the first window
    comes with the notification. Started from the app grid, it does not.

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
  activity. A vertical USB stick (connector with two holes, filled body)
  using the icon's whole height, so it reads at 16–22 px.
- KDE, XFCE etc. get the GNOME theme; refine later if users ask.
- Never use a non-system font. No web fonts, no downloads.

## Receiver side

The window's left column has a second group, **Received** (step 7b): the
sticks someone sent this device, each with its name (the sender's, shown
isolated so its writing direction never spills over) and what it is doing
in a few words: `Downloading 45%`, `Up to date`, `Up to date so far`,
`Waiting`, `Paused`, `Stopped`. Below it, `+ Add a sharing link…`.

- **Add a sharing link:** a field to paste the link (never shown back,
  logged or put in a message: it holds the stick's key) → `Look at it` →
  "Looking for the sender… please wait." (at most half a minute) → the
  field and its buttons go (they have done their work), and the stick's
  name comes, then "What to get:" with `All` / `None` and every
  top-level folder and file with a tick (all ticked) and its size, largest
  first; a top-level folder opens with an arrow (`▸`) to show its folders
  and files one level down, indented, each with its own tick; a stick of
  one folder is shown open; a folder with some of its inside un-ticked
  shows a part tick (ADR 0012, Amendment 1); "New things the sender adds will come too."; `Save to <folder>`
  (Downloads/<stick name> at first) with `Choose…` (the system's folder
  picker); and `Needs up to X · fits` (the ticked ones, as they change), or
  "Needs up to X, and only Y are left where it would be saved. Un-tick
  something, choose another place, or free some room.", or "Tick at least
  one thing.", or "The sender is still counting its files: more is coming."
  (or, if what it has counted already does not fit, "Needs at least X so
  far, and only Y are left…", and Get it is off)
  → `Cancel` / `Get it` (ADR 0012). The list scrolls by itself when long;
  what is below it, with `Get it`, stays in sight.
  If the sender cannot be reached: "The sender's computer is off, or
  OurStick is not open there. Try again when it is." A link to one of this
  device's own sticks, or one it already receives, is refused, saying so.
- **A received stick:** its name, `Saved in <folder>` with `Open folder`,
  and what it is doing in a sentence: "Downloading: 1.2 GB of 2.6 GB" with
  a bar; "Up to date. New files from the sender come by themselves while
  OurStick is open."; while the sender still reads its folder and all it
  has said so far is here, "Up to date so far. Anything more the sender
  adds will come by itself." (not "getting more": nothing is being fetched); "Waiting since 14:30. The sender's computer is off,
  or OurStick is not open there. It goes on by itself when they are back."
  (or why else it waits: no room, its folder's disk unplugged, the sender
  no longer sharing it with this link); "Paused: …"; or, in red, why it
  stopped, with `Try again`. Under it, what it leaves out, grouped by the
  folder it is inside: "Not getting: Films; MUSIC and Pictures (inside
  Rescued_Disk)." and "Not updating: Music (it stays in the folder)." Its `⋯` menu:
  `Pause` / `Resume`, `Choose what to get…` (the same ticks, the same tree,
  a folder opening while the sender is away too, from the latest list; un-ticking
  what is here says "Music stays in your folder, but is no longer updated.
  OurStick never deletes anything."; a name the stick no longer has is
  marked "not on the stick now", at either level; `Save`), `Forget this stick…` (red): "It
  is no longer kept up to date. Its files stay in <folder>." When there is
  not room, its wait says to leave something out with `Choose what to
  get…`.
- Under the received sticks: "Senders see you as: a Mac. `Change…`" (or
  "Maria (Mac)") — the one place a receiver may give a name. `Change…`
  opens "Your name for senders (optional)" with `Cancel` / `Save` and
  "Leave it empty to show only a Mac. Your computer's own name is never
  sent."; at most 40 characters, none invisible. Never asked for by a
  popup or a step. Not a settings page (there is none in stage 1).
- Received sticks are kept up to date in the background while the app
  runs, each going on where it was after a restart; files the sender
  removes stay here; a folder whose disk is unplugged is never filled on
  another disk. Quit names the sticks coming in: "Getting “Holiday” stops;
  it goes on where it was next time."
- **First launch** has a second way in: "Someone sent you a sharing link?
  `I got a sharing link`", straight to adding it, with no stick of one's
  own.
- Not yet: choosing deeper than one level inside a top-level folder (ADR
  0012, Amendment 1: a folder inside an opened folder is chosen whole), and
  receiving without the app (a web receiver is out of v1). `ourstick get`
  still works, but never touches a stick the window receives.

## Signing and relays

- The macOS app is signed with an Apple Developer ID and notarized
  (2026-10-08), with the keys in the data protection keychain, which never
  asks (ADR 0021).
- **The Windows installers are not signed yet** (MSI and NSIS): SmartScreen
  warns of an unknown publisher on download and on first start.
- Relays: n0's public ones; our own (ADR 0006) only if they are not enough.

## Open questions

- **"X GB remaining"** for the owner: needs a small addition to the protocol,
  where the reader tells the owner how much it still needs.
- **macOS mount point:** the image is attached under `/Volumes` (where Finder
  shows volumes), so its path can change (`OurStick 1`); the app finds the
  stick by its image and records the folder's path at every mount. Whether
  Finder lists it in Locations with an eject button is checked by eye in the
  manual round.
- **Windows drive name:** the `subst` letter shows under the system drive's
  label ("Windows (O:)"); neither the per-user nor the machine-wide
  `DriveIcons` label applies to a `subst` drive on Windows 11 24H2 (tried).
  The stick's name is shown by its entry in Explorer's navigation pane
  instead (ADR 0023); only a VHDX would name the letter itself.
- **`ksni` upstream:** once a `ksni` release has
  `Tray::provide_xdg_activation_token` (pull request #43), drop
  `third_party/ksni` and the `[patch.crates-io]` entry.

## Out of scope (stage 1)

FUSE or any virtual filesystem, right-click "Send to" in file managers,
permissions, expiry, revoking a link other than by changing it, a settings
page (the one setting, start at login, is in the tray and About), and the
app updating itself (planned next). Many sticks, folders linked without
copying (ADR 0008) and the receiver window, once out of scope here, are
built.
