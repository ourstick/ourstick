# OurStick — GUI spec (stage 1: desktop)

The window as ADR 0028 makes it: two verbs, Send and Receive, and files sent
from where they are. The mockups of ADR 0028 ("OurStick, simpler") show every
screen and case; where they and this file differ, this file and the
decisions are right. `gui-mockup.png` and `gui-mockup.html` are history: the
first window, of one stick copied onto like a drive. Anything not in this
file is out of scope for stage 1.

## Principle

Someone who uses a computer for email and photos, and nothing more, sends and
receives without help. Every screen has the fewest elements it can. No
settings page.

**Words.** The window never says stick, drive, mount, eject, linked, sharing
link, capacity or manifest. "Link" is only the address
(`ourstick.com/s/#…`); a thing is called by its name. The verbs are Send,
Receive, Download, Stop sending, Resume (a stop by the user, on either side)
and Remove. "Stick" stays the code's and the docs' word.

**Messages** are grey (good to know, nothing to do), amber (it works, but
something could be better) or red (stopped, needs you). Each says what to do
next, usually with a button. State is never told by colour alone: a symbol
goes with it (↑ ✓ ↓ ! ⏸ ✕ …).

## Nothing is copied

What is sent is sent from where it is: every folder or file chosen is a
linked entry (ADR 0008 §2), never a copy. A stick made now has no folder of
its own (`has_own_folder = 0`): its root is an empty folder in the data
folder, `<data>/sent/<stick id hex>`, which nothing writes to and the window
never shows, and it has no drive, marker, letter, bookmark or image.

**One place decides what may be sent**: `places::may_send` in the core. It
refuses the home folder and anything holding it; private folders and what is
in them (a top-level dot folder of the home, `~/Library`, `AppData`, what has
the Windows hidden or system attribute); the system's folders, per platform
(ADR 0028 §2 has the lists); OurStick's own folders; a symbolic link or a
junction; and what is inside, or holds, something already chosen. It asks
first about a main folder of the home (Documents, Desktop, Pictures,
Downloads, Music, Videos, by the system's list) and a whole disk. Two entries
with one name (ignoring case) are refused, never numbered
(`LinkError::NameTaken`).

The rules of linked entries hold as before: symbolic links inside are not
followed; a gone entry (moved, its disk unplugged) keeps its last version
offered, and comes back by itself when its path is there again; files in
constant use are left out until still; hidden files that systems and file
managers make (`.DS_Store`, `._*`, `.Trashes`, `.directory`, …) are never
sent (`docs/protocol.md`, "Owner-side rules").

### Sticks made before ADR 0028

They keep what they had (`has_own_folder = 1`): their own folder, sent as
before, its entries listed with the linked ones; and their drive, made at
each start (macOS: the sparse image attached; Linux: `~/OurStick/<name>`
bookmarked; Windows: `%USERPROFILE%\OurStick\<name>` as a `subst` letter
with its entry in File Explorer's pane, ADR 0023). The window shows no Mount
or Eject. What is put into the folder in a file manager is sent. Nothing
dropped on the window is copied into it any more: it is linked. When the
folder is not there (an ejected image, a deleted folder, an unplugged disk),
an amber line above its list says "Some of its files are on a disk that is
not attached." with `Find it…` (attaching the image on macOS); the last
version stays offered.

## Screens

### Home

- `Send` (blue, "Choose folders, get a link to send.") and `Receive`
  ("Someone sent you a link? Open it here."), big, side by side.
- **You are sending** and **You received**: one row per thing, its name and
  one sentence with its symbol: "↑ Maria is getting it now: 60%", "✓ 2
  people have all of it", "! A folder is missing", "⏸ Stopped by you",
  "↓ Downloading 45%", "✓ All here. New files come by themselves.",
  "! Waiting: the disk is full", "✕ Stopped: needs you", "… Waiting for the
  sender". The whole row opens its view (›). A row has at most one button:
  the next step when it needs the user (`Find it…`, `Resume`, `Make room…`,
  `Try again`), or `Open folder` on a received thing that is all here.
- Empty, the first start shows the two buttons only, under "Welcome to
  OurStick" and "Send folders of any size straight from your computer to
  someone else's. No account. Only the people you send the link to can see
  them.", with the start-at-login line and its check, and `OK` (ADR 0024).
  It makes nothing: the keys are made at the first Send or Receive.
- A problem with the whole app (the password store does not answer) is one
  red box at the top: "OurStick cannot open its keys" with what it means,
  `Try again` and `Report a problem`; rows say "Waiting for the keys".

### Send

1. **"What do you want to send?"** Drag here, or `Choose folders…` /
   `Choose files…` (Linux's and Windows' pickers do not take both at once).
   Each chosen thing is listed with its icon, size ("counting… 12,400 files
   so far" while counted) and `×`. A refusal stands in the list in its place,
   in its words, with its `×`: "“Summer” is inside “Holiday 2025”, so it
   comes along already.", "“OurStick” is OurStick's own folder and cannot be
   sent.", "“Windows” is part of the system and cannot be sent.", "“.ssh”
   holds private settings and cannot be sent.", "Your whole home folder
   cannot be sent. Choose folders inside it.", "Two folders are called
   “Photos” (from Pictures and from the disk “Expansion”). Rename one
   first, then add it again." A main folder or a whole disk asks first
   (below). In blue under the list: "Nothing is copied: OurStick sends them
   from where they are. Leave them there. If you change a file, they get the
   change too." Then **Name, as others will see it** (1–60 characters),
   filled in from the first thing chosen ("Filled in from the first folder.
   You can change it."); a name another thing this computer sends has:
   "You already send something with this name. Choose another." `Get the
   link`.
2. **The link.** "“Holiday 2025” is ready to send", the link hidden but for
   its start (`ourstick.com/s/#•••`), `Copy link` ("Copied ✓" once done),
   and in amber: "Anyone who gets this link can take everything in it, also
   the files you add later. Send it only to people you trust." Three steps:
   "Paste it in an email or a message." "They need OurStick too. When they
   click the link, the page shows them how to get it." "Keep this computer
   on and OurStick open. They get what you add or change, until you stop
   sending." `See who gets it ›`.

It is published at once, while its folders are still counted ("Getting it
ready… 12,400 files so far"; "People with the link can start already: the
rest follows by itself.").

Made from exactly one folder, its contents are what is sent, at the top: a
receiver gets `Downloads/Holiday 2026/Beach/…`, not the folder inside a
folder of the same name (ADR 0028 §2, amendment of 2026-10-20).

**Asking first** (one dialog, the safe choice blue):

- "Send all of “Documents”?" "This is one of your main folders: 8,240 files,
  46 GB. Folders like this often hold private papers, like bank letters, tax
  forms or passwords." In bold: "Whoever gets the link sees all of it, and
  every file you put in “Documents” later." `Send all of it` (red outline) /
  `Choose a smaller folder`.
- "Send the whole disk “Expansion”?" "That is 1.4 TB. It can take days, and
  this computer must stay on the whole time." "Usually you want one folder
  on it, not all of it." `Send all of it` / `Choose a folder on it`.

### Something you send

- `‹ Back`, its name, `Copy link` (here and on the link screen only; never
  on the home or the tray) with, in amber under it, "Anyone who gets the
  link can take everything in it, also what you add later.", and `⋯`.
- Its status box, who has it (ADR 0015), one line per computer, in the
  receiver's own words, never a computer's own name: "Nobody has opened the
  link yet." (with `Copy link`); "Maria is getting it: 60%" with a bar and
  "about 4 minutes left"; "A Windows PC is getting it: preparing a big file
  first…"; "3 people have all of it, up to date." with `Who?`. The full
  list: "Maria (Mac) has all of it", "A Windows PC has the part it chose",
  "A Linux PC had all of it on 3 Oct, not what changed since", "A Mac is
  waiting: it does not fit there", "A Mac (since 3 Oct) is waiting: the
  disk it saves to is not plugged in", "A Windows PC paused it", each with
  `×`; "and 12 more", `Clear this list`, "Each computer says this about
  itself."
- **What is in it**, with `+ Add more` (the same picker, rules and
  questions as Send): each entry with its icon, size and where it is ("2.8
  GB · Pictures"; a click shows it in the file manager) and `×`, which asks
  "Stop sending “Summer”? Those who have it keep it." The one folder a
  stick was made from has no `×`, and under it in grey: "This is what you
  send. To stop, use ⋯ › Stop sending."; the `+ Add more` menu then says
  "They go next to what is in “Holiday 2026”." An old stick's own
  entries are listed with the rest; one of its own entries with a linked
  entry's name is hidden (`catalogue.rs`). Under the list: "Leave them where
  they are. If you change a file, they get the change too. If you move or
  delete one, it stops coming; what they have stays with them."
- Notes, under the status, each while it holds:
  - grey: "Sending slowly, so another program can use the disk." / "Reading
    your folders slowly, so another program can use the disk." (ADR 0013,
    0017); "Changes in some folders reach the others within a minute."
    (watches exhausted);
  - amber: "This is slower than it could be" with "Maria's computer cannot
    reach this one directly, so it takes the long way. Allowing OurStick in
    Windows Security makes it faster." and `Show me how`; on macOS with the
    Local Network refused, "Maria is in your home, but your Mac does not let
    OurStick reach her. Turn on OurStick under Local Network." and `Open
    Settings` (ADR 0020); a gone entry: its name, "Not found. Was it moved,
    or is its disk unplugged? Those who have it keep it." and `Remove it`;
  - "Stopped by you": "The link does not work until you start again.
    Nothing was deleted." `Resume`;
  - red: "Stopped: the network did not start" ("Is this computer on the
    internet? Check, then try again.") or "Stopped: OurStick is open twice"
    ("Another OurStick on this computer has it. Close that one, then try
    again."), each with `Try again`;
  - while it changes, the buttons wait: "Renaming…", "Making a new link…",
    "Stopping…", "Removing…".
- `⋯`: `Rename…`; `Make a new link…` ("The old link stops working."); `Show
  the files on this computer` (old sticks only); `Stop sending…` ("You can
  start again later. The link stays.") or `Resume`; `Remove from
  OurStick…` (red, "Your files stay where they are.").

**Its dialogs** (each with "… please wait." while it works):

- Rename: "New name", "Others see the new name when their copy next
  updates. The link stays the same." `Cancel` / `Rename`. For an old stick,
  its folder or volume, bookmark or letter follow, as before.
- "Make a new link for “Holiday photos”?" "The old link stops working, for
  everyone. Send the new one only to the people who should still get it."
  "Use this if the link reached someone it shouldn't have." `Cancel` /
  `Make a new link`. The only way to take access back (ADR 0009 §5).
- "Stop sending “Holiday photos”?" "The link stops working until you start
  again. Your files stay where they are." In bold, if so: "Maria is getting
  it right now." `Keep sending` / `Stop` (red).
- "Remove “Holiday photos” from OurStick?" "Its link stops working for good.
  Your files stay where they are on this computer, and those who got them
  keep their copy." `Cancel` / `Remove` (red). Remove never deletes a file
  of the user's. For an old stick its drive goes (letter and pane entry,
  bookmark, the image detached), its marker is deleted, and the dialog says
  where its files stay: "Its files stay in OurStick › Holiday photos.
  Delete them yourself when you no longer need them." (macOS: "in the file
  Holiday photos.sparseimage", with `Show it`).

### Receive

- **From the link page** (ADR 0030): "Open in OurStick" copies the link
  and opens `ourstick:receive`, which carries nothing; OurStick comes to
  the front and opens Receive (not over a dialog, a receive under way or
  Quit). On Linux and Windows, coming back to Home after the window lost
  focus also opens it, for a link not seen before of a stick this computer
  neither sends nor receives. A link found either way shows "✓ We found a
  link you copied", "Someone sent you files", "OurStick looks for the
  sender's computer once you open it.", `Not this one` and `Open it`:
  only `Open it` makes it look.
- **Receive** reads the clipboard on that press, and again each time the
  window comes back while "Paste the link you got" waits with its field
  empty (ADR 0028 §4, amended), never otherwise but as above (`link_on_clipboard`: an
  OurStick link or nothing; never logged, shown or put into an error; the
  clipboard is not cleared). A failed read (a system
  that asks, Wayland without focus) falls back to the field, silently.
- **A link found:** "✓ We found a link you copied", then "Finding the
  sender's computer…" ("This takes up to half a minute.", `Cancel`), then
  its name, big, "2 folders and 1 file · 3.2 GB · it fits on this
  computer", "It goes to Downloads › Holiday photos" with `Change…`, `I want
  only some of it…`, "Not this one? Paste another link", and `Download`
  (blue, bottom right).
- **No link:** "Paste the link you got": "In the email or message you got,
  copy the whole link. It starts with ourstick.com/s/" and "Come back here:
  OurStick finds it by itself. Or paste it below.", the field, `Open`, and
  "Clicked the link and a web page opened? Press “Copy the link” on that
  page, then come back here."
- **Only some of it** (ADR 0012): "What do you want from “Holiday
  photos”?", `All` / `None`, every top-level folder and file with a tick
  and its size, largest first; a folder opens (`▾`) one level down, with
  part ticks (ADR 0012, Amendment 1); "1.9 GB · it fits", "To Downloads ›
  Holiday photos · Change…", `Download`.
- **When something is in the way**, in place of the card:
  - "This link is not whole. Copy all of it from the message: it is long.";
  - "You already get “Holiday photos” on this computer." / "This is the
    link to “Tutu Miles”, which you send yourself." with `Show it`;
  - "The sender is offline right now" "You can wait: it comes by itself,
    all of it, once they are back." `Cancel` / `Try again now` / `Wait for
    it` (ADR 0025; not "computer": a sender may be a phone, ADR 0029);
  - "It does not fit: only 1.1 GB are free." "Take only some of it, save it
    on another disk, or make some room." `Only some of it…` / `Another
    place…`, `Download` off;
  - "The sender is still counting, so more is coming. You can start now:
    the rest follows." with what is counted so far;
  - red: "The network did not start. Is this computer on the internet?
    Check, then try again." `Try again`; "What came does not match this
    link. Ask the sender to send you a new link."

### Something you received

- `‹ Back`, its name, `Open folder`, `⋯`; its status box; under it what it
  leaves out.
- Status: "Downloading… 38%" with a bar, "1.2 GB of 3.2 GB", "You can close
  this window. It goes on by itself, also after a restart." and "When the
  sender adds or changes files, you get them too."; "All here" ("New files
  from the sender come by themselves."); "All here so far" ("The sender is
  still counting a big folder. The rest comes by itself."); "Waiting for the
  sender · 1.2 GB of 3.2 GB here" ("Since 14:30 their computer is off, or
  OurStick is closed there. It goes on by itself."); "Waiting: the disk is
  full" ("Make some room, or take only some of it. It goes on by itself."
  `Only some of it…`); "Waiting: its disk is not plugged in" ("Plug in “My
  Passport”. It goes on by itself."); "The sender stopped this link" ("Ask
  them for the new link. What you have stays in your folder." `Remove…`);
  "Paused · 1.2 GB of 3.2 GB here" `Resume`; red "Stopped: a file could not
  be saved" ("Something on this computer failed. What came so far is
  safe." `Try again`).
- A link waited for (ADR 0025): "Waiting for the sender since 13 October"
  ("Its name and files appear once their computer is on. Leave OurStick
  open: minimize it, don't quit."); after 30 days, "The sender did not
  answer for 30 days" ("Ask them to open OurStick, or for a new link."
  `Remove…`).
- What it leaves out, grey: "Not getting: Films, and Mountains (in Holiday
  2025). No longer updated: Music (it stays in your folder)." `Change…`.
- `⋯`: `Pause` ("Then “Resume” here, when you want.") or `Resume`; `Choose
  what to get…` ("Take some folders out, or add them back."; un-ticking
  what is here says it stays but is no longer updated; a name the sender no
  longer has is marked "not there now"); `Move to another folder…`; `How
  the sender sees you…`; `Remove…` (red, "What you have stays in its
  folder.").
- "Remove “Family videos”?" "New files from the sender stop coming. What
  you have stays in Downloads › Family videos." "To get it again, open the
  link again." `Cancel` / `Remove`.
- "How senders see you": "Your name (you can leave it empty)", "Senders
  will see “Maria (Mac)”. Empty, they see only “a Mac”. Your computer's own
  name is never sent." At most 40 characters, none invisible. `Cancel` /
  `Save`. Also in About.
- Received things are kept up to date while the app runs, each going on
  where it was after a restart; files the sender removes stay here; a
  folder whose disk is unplugged is never filled on another disk.

### The link page in a browser

`ourstick.com/s/#…` with no app: "Someone sent you files", "They come
straight from their computer to yours. To get them, you need the free
OurStick app." Step 1, "Get OurStick and open it.", `Download for <this
system>` and "Mac or Linux" (the others), "Already have it? Skip to step
2."; step 2, "Copy the link, then press Receive in OurStick. It finds the
link by itself." with `Copy the link` ("Copied ✓ Now open OurStick"). "On a
phone? OurStick is only for computers for now." (until ADR 0027), and "This
page never sees the files or the key: they stay in the link, on your
device." The fragment never leaves the browser (ADR 0022).

### Closing the window

- **✕ never quits.** Windows and Linux: it minimizes the window to the
  taskbar or dock, never hides it (see GNOME's activation token under Known
  limitations). macOS: it closes the window; OurStick stays in the Dock,
  and a click there brings it back. Vanilla GNOME has no minimize button:
  the window is found with the ⊞ key, Alt+Tab or the app grid.
- **The first ✕ only**: "OurStick is still open", with a picture of this
  computer's screen: the app's icon at the bottom (on the left on Ubuntu)
  marked "Click here", the small icon near the clock marked "Or here". "To
  open it again, click its picture at the bottom of the screen." and "It is
  also near the clock. If you do not see it there, click ^." (Windows) / "It
  is also at the top of the screen, near the clock." (macOS) / "It is also
  at the top right, near the clock." (Linux with a tray). Linux without a
  tray host: the ⊞ key and the window among the others, "To open it again,
  press the key with the Windows logo ⊞, then click OurStick." Then "What
  you send keeps going while it is open. If you quit, sending and receiving
  stop until you open it again." `Quit OurStick` / `OK`. `OK` sets
  `settings.still_open_told`; `Quit OurStick` does not. The window's own
  `–` shows nothing.
- **The picture** is chosen each time, nothing cached. On Linux a tray host
  is there when `org.kde.StatusNotifierWatcher` has an owner on the session
  bus and `IsStatusNotifierHostRegistered` is true.
- **Quit** (the tray, the app menu, the Dock and ⌘Q on macOS, `Quit
  OurStick` above): "Quit OurStick?" "While it is closed, nothing is sent
  or received. It all goes on when OurStick opens again." In bold, what
  goes on: "Maria is getting “Holiday photos” right now.", "“Family videos”
  is still downloading." The start-at-login check. `Quit` / `Minimize
  instead` (blue).

### Tray

- Always there while the app runs, a green dot while someone gets
  something. The tray is a help, never the only way: many never see it.
- Menu: `Open OurStick`; each thing with its sentence ("↑ Maria is getting
  it", "✓ 2 people have all of it", "↓ Downloading"), a click opening the
  window on it; `Send…`, `Receive…`; `About OurStick`, the `Open when the
  computer starts` check, `Quit…`. Made again when a status changes, at
  most once a second.
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
  sends and receives; the window opens anyway for the keys' move or the
  Local Network step. The check is in four places: the first start, About,
  the tray and the Quit dialog. The system's own "off" (Task Manager,
  Startup Applications) is kept; the check shows it where it can be read
  (Windows, Linux).
- About: the icon, name, version and tagline; ourstick.com, Privacy, Report
  a problem; the start-at-login check; "Others see this computer as a Linux
  PC · Change…"; "To open OurStick again, click its picture:" with the
  same picture as the first ✕; the copyright line; OurStick's licence and
  the libraries' licences (the installed files). From the tray and the
  macOS app menu.

### macOS: first start and the keychain

- The Local Network permission (ADR 0020): at the first start, before
  anything binds, "Your Mac will ask you something" "Choose Allow. Then
  OurStick can send straight to computers in your home, much faster." "It
  asks only once. You can change it later in System Settings." `Continue`.
  Shown once per device.
- Keys moved from an old version (ADR 0021): "One last password" "Your Mac
  will ask for your password up to 3 times. Each time, type it and choose
  Always Allow. After this, it never asks again." `Continue`; if refused,
  "The password was not accepted." "Nothing changed: everything works as
  before. You can try again now or next time." `Not now` / `Try again`.

## Semantics that must hold

- **Stop sending** is the only thing that stops serving a thing; Quit stops
  all of it. Both are always confirmed.
- **Remove never deletes the user's files**, on either side.
- **The link is the key.** Never logged, never shown whole in the window
  but in its field to copy, never sent anywhere by the app itself.
- **Relay policy (ADR 0006 v2):** never stop, never ask. Without a direct
  path, data goes through the relay at a limited speed, with a note that
  stays while it does.
- **Sleep:** while someone is getting something, system sleep is inhibited
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
- macOS (sticks made before ADR 0028): a sparse image does not shrink when files are deleted; it gets
  smaller only after `hdiutil compact` while detached.
- Linux (sticks made before ADR 0028): the bookmark's icon is up to the file manager; Nautilus and Thunar
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
    opens a new window (Send…, Receive…, Quit…) first brings the main
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
  supports them; fall back to the table. On macOS the text on the accent is
  white: WebKit gives `AccentColorText` as black there.
- Icons: symbolic/monochrome line icons (Adwaita-style on Linux, Fluent-style
  on Windows, SF-Symbols-like on macOS). One icon set with per-platform
  stroke weight is enough; do not ship three sets.
- Tray icon: monochrome, follows the panel theme (light/dark), green dot for
  activity. A vertical USB stick (connector with two holes, filled body)
  using the icon's whole height, so it reads at 16–22 px.
- KDE, XFCE etc. get the GNOME theme; refine later if users ask.
- Never use a non-system font. No web fonts, no downloads.


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
- **Sticks made before ADR 0028** keep their drive with no way to eject it
  from the window. If people miss Eject, or the drive confuses them now that
  new sticks have none, a later step may move them to plain folders.
- **`ksni` upstream:** once a `ksni` release has
  `Tray::provide_xdg_activation_token` (pull request #43), drop
  `third_party/ksni` and the `[patch.crates-io]` entry.

## Out of scope (stage 1)

FUSE or any virtual filesystem, right-click "Send to" in file managers,
permissions, expiry, revoking a link other than by changing it, a settings
page (the one setting, start at login, is in the tray and About), the
app updating itself (planned next), copying onto a stick (gone with ADR
0028), and languages other than English (after ADR 0028, with the strings in
one place).
