# Roadmap and open work

What comes next, what is left to try by hand, and what waits on others.
The reasons behind the design are in `docs/decisions/`; the limits of the
code in `docs/protocol.md` ("Known limits") and `docs/gui-spec.md`.

## 0.3.0

- [x] **A new version, told (ADR 0033):** the signed `version.json`, the
      line on Home and in the tray, About's setting. Tried with a test
      notice in the Windows VM, on Linux (the tray's line too) and on macOS
      (2026-11-05): a look, a changed notice refused, no connection, the
      setting off and on.
- [ ] Still to try: a start at login with no window. A proxy (HTTP and
      HTTPS, `tests/version_proxy.rs`) and a captive portal's page or
      redirect (`new_version` tests) are covered by tests.
- [x] **Keys wiped from memory when dropped** (`zeroize`); what is not
      wiped yet is in `docs/protocol.md` ("Known limits").
- [x] **Hardening from the security reviews:** bounds on what a peer may
      send or hold (ADR 0031), received files marked as from the internet
      (ADR 0032), shared files opened without following links, and the
      SHA-256 sums signed with minisign.
- [x] **The release** (2026-11-05): the three installers with their
      signed SHA-256 sums on the site and in GitHub Release v0.3.0, and the
      first `version.json` (serial 1, newest and security 0.3.0). Live sums
      and signatures checked from outside; the Windows installer tried over
      0.2.1 in the VM.

## 0.2.1

- [x] **From the link page to the app (ADR 0030):** "Open in OurStick" on
      the link page, and a link copied seen when the window comes back.
      Tried by hand on Linux and Windows (Firefox and Chrome).
- [x] **Quit within reach:** `Quit OurStick…` on Home, and on Linux a tray
      icon that comes when the panel does.
- [x] **The release:** the three installers with their SHA256 sums on the
      site, with the link page, the new texts and the new demo video
      (2026-11-04); GitHub Release v0.2.1.

## 0.2.0

- [x] **A simpler window (ADR 0028):** Send and Receive, nothing copied;
      one folder sent is the stick itself; what is received goes to
      Downloads, and "Move to another folder…" moves it. Tried by hand on
      Linux, Windows (VM) and macOS (2026-10-20), both ways.
- [x] **The release:** the three installers with their SHA256 sums on the
      site, a new demo video (2026-10-21); GitHub Release v0.2.0.

## 0.1.3

- [x] **The window opened after a start at login, on Linux:** it was blank
      (white) in 0.1.1 and 0.1.2. The start at login itself works on all
      three systems (Linux tried on 2026-10-15: started hidden, the sticks
      shared again).
- [x] **The release:** the three installers with their SHA256 sums on the
      site (2026-10-17); GitHub Releases once the repository is public.

The Linux package is `ourstick` from 0.1.1 (it was `our-stick`): it
replaces 0.1.0 when installed. Opened as a downloaded file, Ubuntu's App
Center shows a `.deb` by its package name, with no icon, publisher or
licence; nothing in the package changes that (only the Snap Store does).

## Next

- [ ] **Updates on Windows and macOS:** today an installed app never updates
      itself. Tauri's updater: the app checks a small file on the site,
      downloads the new version signed and installs it. Needs an update
      signing key, kept like the others (if it is lost, no installed app can
      be updated again). An ADR first.
- [ ] **A Snap for Ubuntu:** found by search in the App Center, with an icon,
      a verified publisher and updates of its own. The hard part is
      confinement (the system keyring, `~/OurStick`, the file manager's
      bookmarks). The `.deb` stays for other distributions. An ADR first.
- [ ] **Sign the Windows installers:** unsigned, SmartScreen warns on
      download and on first start.
- [ ] **Room for a download (to decide):** every download claims 2 GiB of
      scratch room besides its own size, however small it is, and leaves
      the system a tenth of the disk (at most 20 GiB). A Mac of 228 GB with
      13 GB free (2026-11-06) refuses every download, a 465 MB stick
      "needs 2.6 GB"; a phone with little room left would too. The scratch
      could be capped at the download's own size, and the system's share
      looked at again.
- [ ] **Windows opens by path (to decide):** a shared file is opened
      through folders held open, but by path, so someone who can write in
      the shared folder can still turn a held folder into a junction in
      place between the checks and the open. Closing it takes opens
      relative to the held folders (`NtCreateFile` with a root directory),
      which is unsafe code (Giorgos's approval), or a check that the opened
      file is the one checked (file id, through a crate already in
      Cargo.lock, a new direct dependency).
- [ ] **macOS: received folders marked too (to decide):** ADR 0032 marks
      received files with the quarantine, not the folders. A received
      folder that is an app bundle (a `.app` inside a stick) then has its
      files marked but not the bundle itself. Marking every folder a reader
      makes would carry the mark onto bundles, as Archive Utility does.
- [ ] **macOS: the concealed mark in the same write (to decide):** "Copy
      link" puts the text, then the concealed type, in two writes; a
      clipboard history app that looks in between would keep the link.
      One write of both takes `NSPasteboardItem` through objc2, unsafe code
      (Giorgos's approval).

## Keys and computers (ADR 0021, rule 7)

- **"Move to another computer":** a stick's keys never move by themselves;
  an explicit action sends them to the new computer and stops the stick on
  the old one, so there is never a second owner. A later ADR.
- **Linux: notice a copied home folder** (the Secret Service's keyring file
  comes along): another machine id than the one the device recorded, so
  ask before sharing its sticks.

## To try by hand

- [ ] **A stick on a full disk:** drop more than fits; the app must refuse
      the copy and say how much is free.
- [ ] **Linux without the usual desktop:** an X11 session, and GNOME without
      the AppIndicator extension. Without a tray the app must say why and
      keep its window reachable.
- [ ] **The relay-only message, for real:** a reader that cannot connect
      directly (both sides behind strict NATs), downloading for over 30 s;
      the app must show how to let OurStick through the firewall. The core
      is tried over the public relays (`tests/relay_only.rs`, ignored by
      default: a reader with no direct path gets the stick, and the owner
      says so after 30 s); still to see the app's line on each system.
- [ ] **A big folder:** about 900 GB in 146 000 files, 10 levels deep: the
      link at once, partial versions every 3 minutes, a restart halfway,
      flat memory, two readers at different places, and paths over Windows'
      260 characters.

## Performance and polish

- **One folder by two paths (Linux, rare):** inotify keeps one watch per
  folder, and notify one path per watch. Two files sent alone from one
  folder reached by two paths (one through a link) share that watch, and
  one file's changes can then arrive under the other path and be taken as
  a neighbour's. Watching the folder as the disk names it, keyed by device
  and inode, would close it.
- **Small files:** a stick of 100 000 small files (464 MB) took 18 minutes
  the first time, Linux to Linux, before ADRs 0014 and 0016 (placing and
  fetching in batches), the sender using 1.24 GB of memory. Measured again
  on 2026-11-06 with 0.3.0's CLI (`tools/measure/reader-memory.sh`), both
  sides on one Linux computer and both folders in RAM: the first catalogue
  about 10 s, the download 13.9 s (34 MB/s), the reader's memory at most
  75 MB and 17 threads, the sender's 87 MB. From a spinning disk the same
  day (084d9e5's CLI; 100 000 files of random bytes, 465 MB, on a 6 TB
  SATA disk, dropped from the page cache first; the reader writing to
  RAM): the first catalogue about 46 s, the download right after it 11.6 s
  (40.7 MB/s, the files still cached by the catalogue); with the catalogue
  kept and the cache dropped again, the download from the cold disk
  15.6 s (30.1 MB/s). The reader at most 86 MB and 17 threads, the
  sender 124–146 MB. Still to measure: to Windows and macOS with the app.
- **The manifest is sent whole** at every new version (about 100 bytes a
  file: 100 MB for a million files). If it matters: send only what changed
  (a protocol change, its own ADR).
- **One read per file (ADR 0011, to write):** today a file is read twice
  before its blob exists, since the nonce prefix comes from the hash of the
  whole file (ADR 0003). A nonce per 64 KiB chunk, derived from that
  chunk's plaintext and index, would encrypt as it reads. Costs: blob
  format version 2, a security review, 24 bytes more per chunk.
- **A parallel catalogue:** one file at a time today; measure two on an SSD
  and on a spinning disk first.
- **Giving way after a restart** (ADR 0017, amendment 1): a new process can
  take the disk for slower than it is and send slowly for a few minutes.
- **Watching very many folders:** past Linux's watch limit the sender checks
  every 60 s instead, walking the whole folder; measure what that costs on
  a spinning disk, and whether to say how to raise the limit.
- **Reading progress:** show "reading 40 %" for a big linked folder, and for
  a big drop before its first partial version.
- **Linking the same folder again** reads it all again; keep a removed
  entry's records for a while and reuse them.
- **Old caches:** remove the folders of ADR 0005 (`cache/<stick>/blobs/`,
  `staging/`), which nothing reads any more.
- **The relay diagnosis on one network:** after a direct path was seen and
  lost, say so, not "a firewall".
- **macOS asks for the Downloads folder** at start, for sticks received
  there; ask only when a stick is first received there.
- **Windows: the drive marker `.ourstick-drive` shows in Explorer;** mark it
  hidden and system.
- **Error words:** a database failure inside some errors (download, space,
  locks, places, linked) shows its own text rather than the plain words.
- **A USB drive icon for the Linux bookmark,** if file managers allow one.
- **The receiver's code:** one lock for every received stick's changes
  (stopping one makes the others wait), and no unit tests yet for its
  pause/resume order; a seam for the app side would allow them.

## Upstream

- **iroh-blobs: an `FsStore` shut down leaves threads behind** (ADR 0018):
  issue and one-line pull request drafted in
  `docs/upstream/iroh-blobs-runtime-drop.md`. Patched meanwhile in
  `third_party/iroh-blobs`; once a release has the fix, delete that folder
  and its `[patch.crates-io]` entry.
- **iroh-blobs: a store on a runtime it is given,** so that a reader's
  batches stop starting and stopping threads; drafted in
  `docs/upstream/iroh-blobs-shared-runtime.md`.
- **iroh-blobs: GC stops for good after any error.** OurStick no longer
  runs its GC; a report still helps others.
- **iroh: a direct path lost and not found again** on one LAN after a pause
  of about 20 s; worked around by connecting again (`docs/protocol.md`,
  "Relay policy"). Worth a report with a small reproduction.
- **ksni: `ProvideXdgActivationToken`,**
  https://github.com/iovxw/ksni/pull/43. Once a ksni release has it, drop
  `third_party/ksni` and its `[patch.crates-io]` entry.

## Later

- **Our own relay** (ADR 0006), only if n0's public relays are not enough.
- **Android:** after the first upload to Play, add Play's app signing key
  fingerprint (Play Console, App integrity) beside the upload key's in
  `apps/website/site/.well-known/assetlinks.json`, and deploy the site.
  The upload key (`ourstick-upload.jks`, made 2026-11-06) is not in the
  repo.
