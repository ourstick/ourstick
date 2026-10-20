# ADR 0028: A simpler window: Send and Receive, files sent from where they are

**Status:** accepted — 2026-10-19. Reviewed by Fable and Sonnet; their
findings are folded in. Changes `docs/gui-spec.md` throughout, and ADR
0008's choice to copy what is dropped and to number clashing names. The
mockups ("OurStick, simpler": the pages Main flows, First run, Sending,
Receiving, Dialogs) are part of this decision; where a sentence here and a
mockup differ, this file is right and the mockup is fixed.

## Context

0.1.3 works: links, big folders, waiting senders, who has it. But the people
who tried it found the window hard. They did not know what a "stick" was
beside a drive, what Mount and Eject did to sharing, why a drop was copied
and could "not fit", which link a "sharing link" was, or that the tray icon
existed (a friend on Ubuntu needed a screenshot to find it). The target is
an older person who uses a computer for email and photos, and nothing more.

Two things carry most of the weight:

1. **Copying.** The stick is a folder of its own, and a drop is copied onto
   it (gui-spec, "The stick is a folder"). Copying brings capacity, "does not
   fit", a copy queue, "already on the stick", Keep both / Replace, the
   drive, Mount and Eject, and "Share it as it is" for what does not fit.
   ADR 0008 rejected "never copy, link everything" because a file would
   vanish from the stick when its original is moved or deleted, unlike a
   real stick. That is true, but it costs every user all of the above to
   protect against one case, which a sentence can explain.
2. **Words.** stick, drive, mount, eject, linked, sharing link, capacity,
   "Senders see you as": each needs a lesson.

The core already shares folders and files where they are (`linked_entries`,
ADR 0008 §2), with the rules that make it safe (no overlaps, symbolic links
refused at the top and not followed inside, gone entries keep their last
version). The change is mostly in the app, with four changes in the core:
names that clash are refused (§2), the places that may not be sent (§2), a
stick without a folder of its own (§7), and Remove keeping the files (§7).

## Decision

### 1. Two verbs, and nothing else on the home

The home has two big buttons, **Send** ("Choose folders, get a link to
send.") and **Receive** ("Someone sent you a link? Open it here."), then two
lists: **You are sending** and **You received**. Each row is one thing, its
name and one sentence of what it is doing, with a symbol as well as a colour
(↑ getting it, ✓ all of it, ↓ downloading, ! needs you, ⏸ stopped by you,
✕ stopped, … waiting). The whole row opens its view (›). A row may have one
button of its own: the next step when it needs the user (`Find it…`,
`Resume`, `Make room…`, `Try again`), or `Open folder` on a received thing
that is all here.

**Words.** The window never says stick, drive, mount, eject, linked,
sharing link, capacity or manifest. "Link" is only the address
(`ourstick.com/s/#…`). The thing is called by its name. The verbs are Send,
Receive, Download, Stop sending, Resume (for a stop by the user, on either
side), Remove. Messages are grey (good to know), amber (works, could be
better) or red (stopped, needs you), and each one says what to do next,
usually with a button.

**First start.** "Welcome to OurStick" makes nothing: no stick, no folder.
The keys are made at the first Send or Receive, as today when a stick is
made; a password store that does not answer is AppProblem's red line.

### 2. Send: nothing is copied

**Send** opens "What do you want to send?": drag here, or `Choose
folders…` / `Choose files…` (two buttons: Linux's and Windows' pickers do
not take both at once). What is chosen is listed with its size (counting
while it counts) and an `×`. Under the list, in blue: "Nothing is copied:
OurStick sends them from where they are. Leave them there. If you change a
file, they get the change too." Then **Name, as others will see it**,
filled in from the first thing chosen, and `Get the link`.

Everything sent is a linked entry (ADR 0008 §2). **One place decides what
may be sent**: a new `places::may_send(path) -> Refuse(why) | Ask(why) |
Fine` in the core, with tests, used by the app (and the CLI if it wants).
Paths are compared after `places::comparable` (case folded on macOS and
Windows). It **refuses**:

- the home folder itself, and anything that contains it (`/`, `/home`,
  `/Users`, `C:\Users`, a disk whose root holds it): "Your whole home
  folder cannot be sent. Choose folders inside it.";
- private folders, and anything inside them: any top-level name in the
  home that starts with `.` (`.ssh`, `.gnupg`, `.config`, `.local`,
  `.mozilla`, `.password-store`…), `~/Library` (macOS), `AppData`
  (Windows), and anything with the Windows hidden or system attribute:
  "“.ssh” holds private settings and cannot be sent.";
- the system's folders, and anything inside them:
  - Linux: `/usr`, `/etc`, `/bin`, `/sbin`, `/lib*`, `/boot`, `/dev`,
    `/proc`, `/sys`, `/var`, `/root`, `/run`, `/opt`;
  - macOS: `/System`, `/Library`, `/Applications`, `/private`, `/usr`,
    `/bin`, `/sbin`, `/etc`, `/var`;
  - Windows: `%SystemRoot%`, `Program Files`, `Program Files (x86)`,
    `ProgramData`, `$Recycle.Bin`, `System Volume Information`;

  "“Windows” is part of the system and cannot be sent.";
- OurStick's own folders (`places::app_places`: the data folder, an old
  stick's folder, §7): "“OurStick” is OurStick's own folder and cannot be
  sent.";
- a symbolic link or a Windows junction at the top (`plan_link` refuses
  symbolic links today; a junction is checked by its reparse point, not
  only by `symlink_metadata`);
- inside, or holding, something already in the list ("“Summer” is inside
  “Holiday 2025”, so it comes along already").

It **asks first** (DlgGuard), the safe choice as the blue button:

- a main folder of the home (Documents, Desktop, Pictures, Downloads,
  Music, Videos, by the system's own list, so their localized names too):
  "Send all of “Documents”?" with its count and size, "Folders like this
  often hold private papers, like bank letters, tax forms or passwords."
  and, bold, "Whoever gets the link sees all of it, and every file you put
  in “Documents” later." `Send all of it` / `Choose a smaller folder`;
- a whole disk that does not hold the home: a mount point (its `st_dev`
  differs from its parent's), `/Volumes/*`, a drive's root `X:\`: "Send the
  whole disk “Expansion”?", its size, "It can take days, and this computer
  must stay on the whole time." `Send all of it` / `Choose a folder on it`.
  Network and removable disks are asked about the same way.

**Names are refused, never numbered.** Today `plan_link` gives a clashing
entry " (2)" (`free_name_where`, ADR 0008 §2); the receiver would not know
which is which. It returns `LinkError::NameTaken` instead, and the list
says: "Two folders are called “Photos” (from Pictures and from the disk
“Expansion”). Rename one first, then add it again." A thing's name taken by
another thing this computer sends ("You already send something with this
name. Choose another.") is decided where it is today, by the store's stick
names. An old stick's own entry with a linked entry's name stays hidden, as
today (`catalogue.rs`).

**The link** (Send2): "“Holiday 2025” is ready to send", the link hidden
but for its start (`ourstick.com/s/#•••`), `Copy link` (which turns into
"Copied ✓"), and in amber: "Anyone who gets this link can take everything
in it, also the files you add later. Send it only to people you trust."
Then three steps: paste it in an email or a message; they need OurStick
too, and the page the link opens shows them how; keep this computer on and
OurStick open, they get what you add or change until you stop sending.
`See who gets it ›` opens its view.

A new stick is published at once, while its folders are still counted
("Getting it ready… 12,400 files so far"), as today.

**Amendment (2026-10-20, Giorgos): one folder sent is the stick itself.**
The round on Windows and Linux found a folder inside a folder of the same
name: "Holiday 2026" sent alone, under its own name, landed in
`Downloads/Holiday 2026/Holiday 2026/…`, though the receiver's screen said
"It goes to Downloads › Holiday 2026". Every linked entry is a top-level
name of the manifest, and the receiver puts the whole stick in a folder
named after it, so the folder's name came twice. It is the common case:
one folder, its name kept.

When `Get the link` makes a stick from **exactly one folder** (the list as
it stands then: not a file, not two things), that folder's contents are the
manifest's top level. The receiver gets `Downloads/Holiday 2026/Beach/…`.
Only the paths in the manifest differ; nothing changes on the wire, and
receivers of every version, 0.1.3 too, get it right. Whatever the name:
"Photos for Anna" made from "Holiday 2026" lands in
`Downloads/Photos for Anna/Beach/…`. The one folder may be a main folder or
a whole disk the user said yes to (Ask), as any folder sent.

- **It is a linked entry, at the top.** The folder is stored as linked
  entries are (its name, path, kind, disk), with `at_top = 1`
  (`linked_entries.at_top`, a new migration): its files are listed without
  its name in front. The stick's own `root_path` stays OurStick's empty
  `<data>/sent/<id>`, so nothing that writes or deletes under `root_path`
  (Remove, the drive code) ever reaches the user's folder. At most one entry
  of a stick is at the top, a folder, on a stick with no folder of its own.
- **Missing or unplugged**, it is as a linked entry: `is_gone` (its disk
  recorded too, so an empty mount point in its place is not taken for an
  empty folder), its recorded files carried, "Not found. Was it moved, or is
  its disk unplugged? Those who have it keep it." in amber, and the stick
  waits for it.
- **Decided once**, when the stick is made, and never converted: turning it
  into a named entry, or back, changes every path, and every receiver would
  get everything again in new places. A stick made before this amendment
  (with one named entry) stays as it is.
- **`+ Add more`** adds named linked entries next to its contents, at the
  top. The menu says so: "They go next to what is in “Holiday 2026”." A
  name that clashes, ignoring case, with one of its top-level names is
  refused: "“Beach” is already in “Holiday 2026”. Rename one first, then
  add it again." While it is not found, nothing can be added ("“Holiday
  2026” is not found, so OurStick cannot check the names in it."). A name
  it gains later with a named entry's name is not sent
  (`is_shadowed`, as for an older stick's own folder).
- **"What is in it"** shows it as one row, like a linked folder: its name,
  its size, where it is (a click shows it in the file manager), but **no
  `×`**, and under it in grey: "This is what you send. To stop, use ⋯ ›
  Stop sending." Taking it out would leave a stick of nothing. It cannot be
  removed through the core either.
- A stick made from one file, or from several things, is as above: each a
  top-level name.


### 3. Something you send

Its view: the name, `Copy link` (only here and on Send2, never on the home
or the tray, and always with the amber line under it), `⋯`, its status (who
has it, ADR 0015, in the same words; "Nobody has opened the link yet." when
no one has), then **What is in it** with `+ Add more` (the same picker and
rules as §2), each entry with its size and where it is ("2.8 GB ·
Pictures": a click shows it in the file manager), and its `×`, which asks:
"Stop sending “Summer”? Those who have it keep it." Under the list: "Leave
them where they are. If you change a file, they get the change too. If you
move or delete one, it stops coming; what they have stays with them."

A gone entry (moved, its disk unplugged) is amber: "Not found. Was it
moved, or is its disk unplugged? Those who have it keep it." with `Remove
it`; on the home row, `Find it…` opens the view. It comes back by itself
when its path is there again, as today.

`⋯`: `Rename…`, `Make a new link…` ("The old link stops working."), `Show
the files on this computer` (old sticks only, §7), `Stop sending…` ("You can
start again later. The link stays."), `Remove from OurStick…` (red, "Your
files stay where they are."). The dialogs are DlgStop, DlgNewLink,
DlgRemove and DlgSmall. **Remove never deletes a file of the user's**: it
removes the link and OurStick's own data for it.

### 4. Receive: the link found by itself

**Receive** calls the existing `link_on_clipboard`
(`receiving/clipboard.rs`: trims, keeps the text only if it is an
OurStick link, else nothing) once, when it is pressed, and at no other
time (today `AddLink.svelte` calls it when it opens; that stays the only
caller, opened only by Receive). The text is never logged, put into an
error or shown (CLAUDE.md); the clipboard is not cleared. Where the system
asks before an app reads the clipboard, or the read fails (Wayland without
focus, from the tray), the view falls back to the field, silently.

With a link: "✓ We found a link you copied", the thing's name, big, once
the sender answers, "2 folders and 1 file · 3.2 GB · it fits on this
computer", "It goes to Downloads › Holiday photos" with `Change…`, `I want
only some of it…`, "Not this one? Paste another link" and `Download`.
Without: "Paste the link you got": two steps, a field and `Open`.

The rest is ADR 0012 and ADR 0025 in new words: "Finding the sender's
computer…", "only some of it" (ReceiveChoose), "The sender's computer is
off right now" with `Wait for it`, "It does not fit: only 1.1 GB are free."
with its three ways out, one's own link or one already received ("Show
it"). Downloading: the bar, "1.2 GB of 3.2 GB", "You can close this window.
It goes on by itself, also after a restart." and "When the sender adds or
changes files, you get them too." The `⋯` menu: `Pause`, `Choose what to
get…`, `Move to another folder…`, `How the sender sees you…`, `Remove…`
("What you have stays in its folder"). Every state is in ReceiveStates.

**Amendment (2026-10-19, Giorgos):** the clipboard is also read each time
the window comes back while "Paste the link you got" waits with its field
empty: the user went to copy the link, and OurStick finds it by itself, as
the screen says ("Come back here: OurStick finds it by itself. Or paste it
below."). Never on another screen, never in the background; only an
OurStick link is taken, as above.

"Senders see you as" leaves the home: it is in About ("Others see this
computer as a Linux PC · Change…") and in a received thing's `⋯`.

### 5. The link page in a browser

`ourstick.com/s/` (apps/website) says "Someone sent you files", why the
app is needed, and two steps: `Download for <this system>` (others under
it), then `Copy the link` and press Receive in OurStick, which finds it.
"On a phone? OurStick is only for computers for now." until ADR 0027. The
page never sends the fragment anywhere (ADR 0022), as today.

### 6. Closing the window, the tray, and Quit

**✕ never quits.** On Windows and Linux it minimizes the window to the
taskbar or the dock, where it is seen and found again (gui-spec already
keeps it minimized, never hidden, for GNOME's activation token). On macOS
it closes the window as every Mac app does, OurStick stays in the Dock, and
a click on the Dock icon brings the window back (as today). Vanilla GNOME
shows no minimize button, so ✕ is its only way; the window is found again
with the ⊞ key, Alt+Tab or the app grid. Today's ✕ asks "Quit or minimize",
and quits without asking when nothing goes on (`dialog.rs`,
`close_requested` and `going_on`); both go.

**Quit** is in the tray, the app menu and the Dock (macOS, also ⌘Q), and in
the dialog below; it opens "Quit OurStick?" (DlgQuit), whose blue button is
`Minimize instead`, and which names what stops.

**The first ✕ only** shows "OurStick is still open" (DlgTray): a picture
of this computer's screen with the app's real icon at the bottom of the
screen (left on Ubuntu) marked "Click here", and the tray icon near the
clock marked "Or here"; "To open it again, click its picture at the bottom
of the screen." and where the small icon is; "What you send keeps going
while it is open. If you quit, sending and receiving stop until you open it
again."; `Quit OurStick` and `OK`. Minimizing with the window's own `–`
shows nothing. `OK` sets `settings.still_open_told`, and it is not shown
again; `Quit OurStick` does not set it. No words like Dock, taskbar or
tray: "its picture", "near the clock".

The picture is chosen each time it is shown, nothing cached: Windows,
macOS, Linux with a tray host, Linux without. On Linux, a tray host is
there when `org.kde.StatusNotifierWatcher` has an owner on the session bus
and its `IsStatusNotifierHostRegistered` is true (Ubuntu's AppIndicator
extension, KDE; vanilla GNOME has none, and the extension can be turned on
later). Without one, the picture is the ⊞ key and the window among the
others in the overview: "press the key with the Windows logo ⊞, then click
OurStick". About shows the same picture, chosen the same way, "To open
OurStick again, click its picture:", for good.

**The tray menu**: `Open OurStick`, each thing with its one sentence
(clicking it opens the window on it), `Send…`, `Receive…`, `About
OurStick`, the start-at-login check, `Quit…`. The sentences follow the
status as the home does, the menu made again at most once a second.

Start at login stays as ADR 0024, its check in four places: the first
start, About, the tray, and the Quit dialog.

### 7. Sticks made before: nothing to do, nothing lost

No dialog and nothing moved. **The update never moves, removes or re-marks
an existing stick's folder**, and its migration only adds columns. At the
first start of this version:

- **Links work as before.** Keys, tickets, the manifest, the protocol and
  the stick tables' rows do not change. The manifest is walked as today
  (`StickLayout::new(root, linked_entries)`). A receiver on 0.1.3 receives
  a stick sent by this version and the other way round: a stick of linked
  entries only is one they know.
- **An old stick's own folder stays where it is** (`~/OurStick/<name>`,
  `%USERPROFILE%\OurStick\<name>`, the disk image on macOS) and goes on
  being sent: its entries are listed in **What is in it** with the linked
  ones. What is put into it in a file manager is sent, as today. Its drive
  (the `subst` letter and Explorer's entry, the Linux bookmark, the
  attached image) is made at each start as today, so nothing the user knew
  disappears; the window no longer shows Mount or Eject. `Show the files
  on this computer` opens it. When the folder is not there (an ejected
  image, a deleted folder, an unplugged disk), one amber line above the
  list says "Some of its files are on a disk that is not attached." with
  `Find it…` (attaching the image on macOS); the last version stays
  offered, as today. New link and Rename work on it as today.
- **New sticks have no folder of their own.** A new column
  `owned_sticks.has_own_folder` (default 1, so every old stick keeps
  what it has; 0 for new ones) decides, in one place, whether there is a
  drive. For a new stick the root is an empty folder,
  `<data>/sent/<stick id hex>` (by id, so Rename never moves it), which
  nothing writes to and the window never shows; `check_new_stick_folder`
  allows exactly that folder under the data folder. With
  `has_own_folder = 0`, the drive parts do nothing: no marker
  (`check_drive` and `settle_marker` are skipped), no letter, bookmark,
  image or Explorer entry, Rename only renames (`rename_owned_stick`, not
  `drive::rename`), and Remove deletes the empty folder.
- **Drops never copy**, also onto an old stick: dragging onto the window
  adds linked entries (§2). The copy queue goes from the app
  (`stick_drop.rs`, its tests, `stop_copy`, and the copy in `going_on` and
  the Quit dialog), and `copy_onto` (`drive/copy.rs`) from the core once
  nothing uses it.
- **Remove of an old stick keeps its files.** Today `finish_removal`
  empties the folder (`empty_drive`) and the app's `take_away` deletes the
  macOS image. Now `take_away` takes the drive away (the letter and
  Explorer's entry, the bookmark, the image detached) but deletes nothing,
  and the hidden marker is deleted, so the folder is a plain folder of the
  user's, which can then be sent like any other. The dialog says where it
  stays: "Its files stay in OurStick › Holiday photos. Delete them yourself
  when you no longer need them." (on macOS: "in the file Holiday
  photos.sparseimage", with `Show it`, which shows it in Finder).

### 8. What does not change

The core's behaviour beyond the four changes above, the transfer format,
and every ADR on the network side. A download in progress is not touched
by the update: the same links, keys and manifests; the sender restarts and
its receivers go on. The CLI is unchanged but for refusing a clashing name
instead of numbering it. Theming per platform, dark mode and keyboard
focus (gui-spec, "Theming") apply to the new screens; the mockups show only
the light Linux look.

## Order of work

1. This ADR, accepted; then `docs/gui-spec.md` rewritten from it and the
   mockups (one file, the window as it is after this).
2. The core: `places::may_send`, `LinkError::NameTaken`, `has_own_folder`
   and `settings.still_open_told` (migration 0026), the drive parts
   skipped for new sticks, Remove keeping files. Tests first.
3. The home, Send and the link (§1, §2), with the refusals and DlgGuard.
4. Something you send (§3), old sticks shown with their own folder (§7);
   the copy queue removed.
5. Receive (§4); the received views in new words.
6. ✕, DlgTray, About's picture, the tray menu (§6).
7. The link page (§5), deployed only when Giorgos says so.
8. A manual round on Linux, macOS and Windows, with an old stick from
   0.1.3 on each (sending, then removed); then a person who has never seen
   OurStick, with no help.

## Consequences

- `docs/gui-spec.md`: "The stick is a folder", the main window, the drop
  zone, the copy queue, capacity, Eject/Mount, the ✕ rule, "Receiver
  side" and "First launch" are rewritten. `docs/gui-mockup.html` is
  history, as `gui-mockup.png` is.
- ADR 0008: "Never copy, link everything", rejected there, is now the
  app's model; §2's " (2)" becomes a refusal. Its §1 (capacity) stays for
  downloads and the space ledger; for the sender there is no capacity any
  more.
- ADR 0023 (the stick by its name in File Explorer) applies to old sticks
  only.
- `docs/schema.md`: `owned_sticks.has_own_folder`,
  `settings.still_open_told`.
- Tests: `may_send` on each platform's list (home, a parent of home, a
  dot folder, `~/Library`, `AppData`, a system folder, the data folder, an
  old stick's folder, a junction, a mount point, a main folder);
  `NameTaken` (also differing only in case); a new stick's root allowed
  and never marked; a new stick renamed and removed without drive calls;
  an old stick listed with its own entries; Remove of an old stick keeping
  its files and image, its marker gone; the clipboard taking only an
  OurStick link; the first-✕ setting.

## Alternatives considered

- **Keep copying, simplify the words.** Capacity, the queue and "does not
  fit" remain, and they are the hardest part to explain.
- **Ask on each send whether to copy.** A question the user cannot answer
  without knowing what it changes.
- **Allow the home folder, leaving out its hidden folders.** It still sends
  every private paper in it, and changes what a walk takes for one case;
  refusing it and asking for its main folders is simpler to say.
- **Move old sticks' files out of their folders and images.** It costs a
  full copy's room on macOS and risks the user's files, to make the old
  look like the new; keeping them where they are costs nothing.
- **A marker in the new stick's empty folder** instead of
  `has_own_folder`: `check_drive` would pass unchanged, but Rename, Remove
  and the drive would still act on a drive that is not there.
- **Hide the window on ✕ and rely on the tray.** The tray is not seen by
  the people this is for, and vanilla GNOME has none.
- **A "What's new" dialog after the update.** It explains a change the
  user does not need to know about; nothing they knew is gone.

## Later

Greek and other languages in the window, with the strings in one place
from the start of this work.
