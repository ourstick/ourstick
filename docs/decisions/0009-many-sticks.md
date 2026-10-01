# ADR 0009: Many sticks per device, each with its own node and name

**Status:** accepted — 2026-09-30. Replaces section 4 of ADR 0007's first draft
("one node for many sticks"). Goes with the mockups of 2026-09-30 ("Many
sticks", in the design canvas "OurStick large shares"). Changes lines of
`CLAUDE.md` listed under Consequences, to be updated when this is accepted.
Reviewed twice before this version.

## Notes from the implementation (step 5, 2026-10-01)

Reviewed again by two other models against the code: the one `share.lock` of
section 2 became four locks with one job each (`share-<hex>.lock`,
`node-key.lock`, `key-file.lock`, and a short `setup.lock` around making the
device or a stick, so two `share` runs of one folder never make two sticks).
A key file is read whole when opened and written whole on each change, so its
lock is held from before it is opened until the program ends, not only around
writes. The stick sharing an older stick's node key is named by trying the
older sticks' own locks, never by writing into a lock file (Windows locks
cannot be read). A device set up from now on records no node key of its own.

## Context

People send different things to different people: a disk for one relative,
family photos for everyone, papers for an accountant. With one stick, one link
reaches everyone, and the only way to cut someone off is a new key for all. The
app should hold several sticks, each with its own link, name and life.

Today: the device has one node key (`SecretName::NodeKey`, its node ID in
`device.node_id`) and one `share.lock`; the CLI already makes one owned stick
per shared folder, all on that node key; the app looks for "the" stick
(`drive_stick()`, found by `drive_size`). Readers use a fresh random node key
on every run (`get`, `ls`). Two endpoints must never run one node key, and one
node key for all sticks tells a reader of two sticks that they come from the
same computer.

## Decision

### 1. A node key per stick, and what happens to older sticks

Each stick made from now on gets its own node key
(`SecretName::StickNodeKey(StickId)`, stored as `stick/<hex>/node-key`), made
with the stick and, like its other keys, never made again (`CLAUDE.md`, rule 9).
`create_stick` stores the keys first, then the row with `owned_sticks.node_id`.
Each shared stick runs its own endpoint, as each `IrohPublisher` does today.

Sticks made before this keep the device's node key (`node_id` NULL), since
their links already carry it. Their rule: **at most one of them is shared at a
time**, enforced by the device lock (section 2). Sharing a second one fails and
names the one already shared; "Give it a new link" moves a stick to its own
node key for good. A new device makes no device node key: `device.node_id`
becomes optional, and `open_device` no longer creates one.

### 2. Locks

Locks are the operating system's file locks (`File::try_lock`): released when
the program ends, so a leftover lock file is harmless. Lock files are never
deleted.

- **A lock per stick**, `share-<hex stick id>.lock`: one program serves a
  stick, and one program changes it (rename, new link, remove).
- **The device lock**, `share.lock`, taken after the stick's lock:
  - by whoever shares a legacy stick (one node key, one program);
  - by every program that writes secrets while the keys are in a key file
    (making a stick, a new link, a removal, and sharing), since the key file is
    not safe to write from two programs at once.
- A program takes its locks **before** reading the stick's row, and asks for a
  key file's passphrase after them, as `setup.rs` does today.

So the app and the CLI can share different sticks at the same time, except
where one node key or one key file would be used twice.

### 3. A name per stick

Each stick has a name the user gives it ("For Maria"): 1 to 60 characters, no
control or format characters, unique on the device ignoring letter case, set in
Unicode NFC. The first launch offers "OurStick", which the user may change
there or later. The CLI names a stick after its folder unless `--name` is
given, adding " (2)" and shortening where needed.

- **Readers see it.** The manifest gets a header, `partial` (ADR 0007) and
  `name`, before its entries: manifest format 2, raised once for both (the name
  empty until this ADR ships). The wire rule is looser than the device rule: 0
  to 60 Unicode scalar values, none of category Cc, Cf, Zl or Zp; a reader
  refuses a manifest that breaks it, like a bad path, and escapes the name
  before any terminal or window shows it. Renaming publishes a new version.
- **The file manager shows it:**
  - **Linux:** `~/OurStick/<folder name>/`, one folder per stick inside one,
    each with its own sidebar bookmark named after the stick.
  - **Windows:** `%USERPROFILE%\OurStick\<folder name>`, each with its own
    `subst` letter; the letter's label stays the system drive's (a limit of
    `subst`), the folder carries the name.
  - **macOS:** each stick its own sparse image, whose volume has the stick's
    name. The image's path is stored with the stick (`owned_sticks.image_path`)
    and never derived from the name or the StickId; new images are named by
    the StickId they were made for.
- **Folder names** come from the stick's name by the same rules as a path in a
  manifest (`docs/protocol.md`, the reader's rules): characters a file system
  refuses replaced, no trailing dot or space, no reserved Windows name (`CON`,
  `NUL`, `COM1`…), compared ignoring letter case and Unicode normalisation;
  on a collision " (2)" is added.
- **Renaming** moves the folder first and publishes the new name second. If the
  folder cannot move (Windows refuses while a file in it is open), the rename
  fails and says to close the files. Linux: the old bookmark is removed and a
  new one added. Windows: the letter is removed before and given again after.
  macOS: `diskutil rename` on the volume.

### 4. Moving the first stick

Today the app's stick is `~/OurStick` itself (`%USERPROFILE%\OurStick` on
Windows); every new stick would sit inside it, and its watcher would publish
them. Before a second stick can be made, the app moves it:

1. Stop its watcher and its endpoint (readers get `NotReady` meanwhile, and
   their paths, relative to the stick, do not change). Windows: remove its
   `subst` letter.
2. Rename `~/OurStick` to a sibling (`~/.OurStick-moving`), make a new
   `~/OurStick`, and move the sibling into it as `~/OurStick/<folder name>`.
   The drive marker moves with the folder.
3. Record the new path, give the bookmark or the letter again (the letter kept
   in `drive_letter`), start sharing.

After a crash the app looks for the stick's marker at the three paths and
finishes the move from where it is. If a step fails (Windows: a file open in
it), nothing further happens, the app says to close the files, and no second
stick is made until the move is done. On macOS nothing moves: the image keeps
its file, and its path is recorded.

### 5. Each stick on its own

- **Stop sharing** stops that stick's endpoint only.
- **Give it a new link**, under the stick's lock (and the device lock with a
  key file): make a new signing key, stick key and node key; write the new
  stick's row, with the same folder, name, drive letter, image path and linked
  folders, and remove the old row, in one transaction (so uniqueness of names
  and folders never sees both); rewrite the drive marker with the new StickId;
  delete the old secrets last. The files, folder, letter, bookmark and image
  are kept. Nothing of the old catalogue is reused (every blob hash changes), so
  the stick is catalogued again (ADR 0007: a full read of every file). Readers
  with the old link are refused ("unknown stick") while the owner is online,
  and see it as offline otherwise; their entry says the link no longer works.
  Readers given the new link check the files they already have by encrypting
  them with the new key (reader work, listed below), and download only what
  differs.
- **Remove this stick**, after a confirmation that names the folder or image
  whose files it deletes, under the stick's lock: mark the row `removing`; stop
  its endpoint and watcher; delete its copied files, only if the folder's drive
  marker holds this StickId (macOS: detach and delete the image, never the
  mount point); remove its bookmark or `subst` letter; delete its secrets;
  delete its rows (`owned_files`, linked paths, the stick) last. At start, a
  row marked `removing` is finished. Linked folders are never touched.
- **Quit** stops every stick; its dialog lists the sticks being shared and
  which have someone downloading.
- **Tray:** one entry per stick with its state and a submenu (Share, Eject or
  Mount, Stop or Start sharing), then "New stick…" and "Quit…". The icon's
  green dot, and the one sleep inhibitor, follow "anyone downloading from any
  stick".

The CLI's sticks and the app's live in the same database; a `kind` column
(`app` or `cli`) says which the app lists, in place of `drive_size` (ADR 0008).
The CLI has no remove or new-link command yet.

### 6. What this does and does not hide

Separate node IDs mean the node ID alone links no two sticks. It is not
anonymity: two tickets from one computer carry the same public IP addresses and
home relay in their address hints, each node publishes the same addresses, and
when one stick keeps the budget busy another's readers wait longer. Who holds
two links, or watches the relay, can still pair them. `docs/protocol.md` says
so.

### 7. Resources

Each stick being shared holds its own endpoint: sockets, a relay connection,
network probes, address publication, a few MB of memory. A stick not shared
holds nothing. Beyond about ten sticks shared at once, the app gives a soft
warning that more cost the computer and the network more.

ADR 0007's working budget is one per program, shared by that program's sticks.

### 8. Receiver

A reader already keeps any number of joined sticks. The receiver window lists
them ("Received"), each with its name from its manifest and its state
(downloading, done and kept up to date, waiting for the sender, link no longer
works). Each link is its own stick.

## Consequences

- Store: `owned_sticks.node_id`, `kind`, `image_path` and a `removing` mark;
  `device.node_id` optional; operations to rename a stick, give it a new
  identity (the one-transaction swap) and remove it; `MemoryStore` and
  `check_owned_stick` with them.
- Migrations, in this order: `kind` (the existing app stick `app`, others
  `cli`), `node_id` (NULL), `image_path` and `removing`; then ADR 0008's; the
  first stick's folder is moved by the app, not by a migration.
- `docs/protocol.md`: the node key per stick and its secret name, the legacy
  rule, manifest format 2 (header layout, name rule) with test vectors in
  `docs/test-vectors`, and a note that an owner on format 2 is unreadable by
  older readers; the privacy limits. `docs/schema.md` with the columns.
- `docs/gui-spec.md`: the stick list, the new-stick dialog, the per-stick
  menu, the tray, the Quit dialog; "multiple sticks" leaves "Out of scope".
- `CLAUDE.md`: "Ένα stick, ένας owner" becomes several sticks per device, each
  with its own link; rule 9 says the keys are per stick, never made again, and
  deleted only when their stick is removed; "several sticks in one ticket"
  stays out of scope.
- The app's state holds several sticks and a sharing state per stick; every
  command that took "the" stick takes a stick id.
- Reader work: checking files already in place against a new link's manifest
  (an encrypt pass with the new key).
- Tests: two new sticks shared at once with different node IDs; two legacy
  sticks refused together; the CLI and the app sharing different sticks; two
  key-file programs refused; a "new link" racing a start (lock, then re-read);
  renaming (folder, bookmark, letter, manifest name) and a rename refused while
  a file is open; a new link (old link refused, new one works, marker
  rewritten, files kept); removal interrupted at each step and finished at
  start; the first stick's folder moved, and interrupted mid-move; manifest
  format 2 names with Cc/Cf characters refused; folder names with reserved
  Windows names and collisions.

## Alternatives considered

- **One node for every stick** (ADR 0007's first draft). One endpoint, fewer
  connections, but any reader of two sticks learns they have one owner, and
  serving several sticks from one endpoint needs a dispatcher of its own.
- **Several sticks in one ticket.** Out of scope in `CLAUDE.md`, and it would
  tie the sticks' access together again.
- **Per-reader keys inside one stick** (cutting off one person without a new
  link for all). Needs identities and key distribution; left for later. A
  stick per person gives the same effect.
- **Give legacy sticks new node keys automatically.** Would break every link
  already sent; the user decides, with "Give it a new link".
- **Linux: `~/OurStick 2`, `~/For Maria` in the home folder.** One folder for
  all sticks keeps the home folder tidy and gives one place to look.
