# To do (not code)

Things that are waiting on something other than code. Code work belongs in
the implementation order in `CLAUDE.md`; limits of the code are in
`docs/protocol.md` ("Known limits") and `docs/gui-spec.md`.

## Launch (after 0.1.1)

- [ ] Make the repository public; a GitHub Release with the three
      installers and their SHA256 sums; archive the GitHub page too
      (web.archive.org, archive.is), as was done for the site on 2026-10-11.
- No social accounts for now (decided 2026-10-11): releases are announced on
  GitHub Releases, users write to hi@ourstick.com, and the name is held on X
  (@OurStick). One account, if people ask for it after the launch.
- [ ] The Hacker News post ("Show HN"), with a first comment on what it is,
      why, and what it does not do yet.

## For 0.1.1 (from the first installs, 2026-10-10)

- [ ] **The Linux package as the App Center shows it:** for a local `.deb`
      file, the App Center (ubuntu/app-center, `local_deb_page.dart`) shows
      the package's name as the title, no publisher, no icon, and the
      licence PackageKit gives (always "unknown" for a `.deb`); nothing in
      the package changes that. The AppStream file (0.1.1) is read once it
      is installed. The package is `ourstick` from 0.1.1
      (`tools/linux/make-deb.sh`; it replaces `our-stick`, tried with apt);
      to see on this computer once "the app here" may restart.
- [ ] **Share stays in sight** when a stick's view is full (fixed on main,
      to be seen in the 0.1.1 builds).
- [ ] **Deploy the site with 0.1.1:** its Windows picture now shows the
      stick's entry in File Explorer's pane, which only 0.1.1 makes.

## Keys and computers (ADR 0021, rule 7)

- **"Move to another computer"**: a stick's keys never move by themselves;
  an explicit action sends them to the new computer and stops the stick on
  the old one, so there is never a second owner. A later ADR.
- **Linux: notice a copied home folder** (the Secret Service's keyring file
  comes along): another machine id than the one the device recorded, so
  ask before sharing its sticks.

## Giving way

- **Giving way when nobody else uses the disk, after a restart** (ADR 0017
  amendment 1, the 20 GB test): "Sending slowly" for 2.5 minutes. A new
  process learns its speed from windows partly in the system's memory
  (the folder read minutes before), so the disk's own speed then looks
  three times slower. Log the windows (`ourstick_core::disk=debug`) and
  see whether learning from the slower of the first windows, or more of
  them, avoids it.

## Memory

- ~~**The reader reaches 200–280 MB**~~ (ADR 0018, 2026-10-06): 114 MB on
  Linux, 109 MB on macOS, 83 MB on Windows for 100 000 small files, and
  its threads no longer grow. If a million files ever matter: ADR 0018's
  A2 (plan and fetch in parts).

## Upstream

- **iroh-blobs: an `FsStore` shut down leaves threads behind** (ADR 0018):
  issue and one-line pull request drafted in
  `docs/upstream/iroh-blobs-runtime-drop.md`, to be sent by Giorgos.
  Patched meanwhile in `third_party/iroh-blobs`; once a release has the
  fix, delete that folder and its `[patch.crates-io]` entry.

- [ ] **iroh 1.2: a direct path lost and not found again.** Two peers on one
      LAN (192.168.1.x): the direct path carried 60 MB/s, the data paused
      about 20 s (the owner preparing a suitcase, before ADR 0017), then the connection moved
      to the relay and stayed there; a new connection went direct at once.
      Also after a reader changed networks. Worked around by connecting again
      (`docs/protocol.md`, "Relay policy"); worth a report with a small
      reproduction.
- [ ] **iroh-blobs: GC logs `error during gc run: Error::Io` after the store
      shuts down, and stops for good after any error.** A public issue; the
      draft (with a standalone reproduction) is kept outside the repository.
      OurStick no longer runs iroh-blobs' GC (ADR 0007; there are no suitcases now, ADR 0017), so it is no longer affected; the report still
      helps others.
- [ ] **ksni: `ProvideXdgActivationToken`,**
      https://github.com/iovxw/ksni/pull/43 (opened 2026-09-29). Answer the
      review if there is one. Once a ksni release has the method, drop
      `third_party/ksni` and the `[patch.crates-io]` entry
      (`third_party/ksni/OURSTICK.md`).

## Before the first public release

- [ ] **Our own relay, `relay.ourstick.com`,** first in the relay list with
      the n0 relays as a fallback (`docs/decisions/0006-relay-policy.md`).
- [ ] **`https://ourstick.com/.well-known/assetlinks.json`,** so that Android
      opens `https://ourstick.com/s/…` links in the app. The file names the
      app's package and the SHA-256 fingerprint of its signing certificate, so
      it needs the release signing key first.
- [ ] The macOS signing and the other items in `docs/gui-spec.md`, "Before
      the first public release".

## After the Windows stage

- [ ] **Stick full, by hand:** fill the stick to its size and drop more; the
      app must refuse the copy and say how much is free (`StickFull`,
      `DriveError::Full`).
- [ ] **Linux without our usual desktop:** an X11 session, and GNOME without
      the AppIndicator extension. The app must not crash; without a tray host
      it logs why and the window stays reachable.
- [ ] **Packages:** `.deb` (with a proper `.desktop` file and icon). The
      `.dmg` (`tools/macos/make-dmg.sh`, signed and notarized) and the
      Windows `.msi` and NSIS installer are made.
- [x] **macOS signing** (2026-10-08): Developer ID, notarized; see ADR 0021.
- [x] **macOS data protection keychain** (2026-10-08, ADR 0021): the keys
      moved, never asked for again.
- [ ] **The relay-only message, seen for real:** a reader that cannot come
      directly (e.g. both sides behind strict NATs), downloading for over
      30 s; the app must show how to allow OurStick through the firewall
      (`docs/protocol.md`, "Reader paths"). Blocking inbound traffic on
      Windows was not enough: hole punching still found a direct path.
- [ ] **A USB drive icon for the Linux bookmark,** if the file managers allow
      one (today Nautilus and Thunar show bookmarks with a folder icon).

## ADR 0007, after step 3

- [ ] **A big folder, by hand:** `share` of the rescued folder (913 GB,
      146,297 files) from the G drive, with a reader on the laptop: the
      link at once, partial versions every 3 minutes, a restart halfway,
      the owner's memory and threads flat (ADR 0017), the cache folder
      (`~/.local/share/ourstick/cache/<stick>/`) holding only outboards,
      two readers at different places.
- [ ] **Remove the ADR 0005 caches** (`cache/<stick>/blobs/` and `staging/`)
      (the suitcases are gone too, ADR 0017): nothing reads them any more, and on
      this PC one holds ~100 GB less than before only because the test cache
      was deleted by hand.
- [ ] **The app shows reading progress:** the watcher reports
      `WatchEvent::Reading`, which the app ignores for now; a big drop gives
      no sign until its first partial version (after 30 s).
- [ ] **The relay diagnosis on one LAN:** `get` said "both sides have
      IPv6, but no direct IPv6 path opened: a firewall…" for two machines on
      the same Wi-Fi whose direct path had just worked. When a direct path
      was seen in this download, say that it was lost, not a firewall.
- [ ] **ADR 0011 (to write): a nonce per chunk, one read per file.** Today
      the nonce prefix comes from the hash of the whole file (ADR 0003), so
      a file is read twice before its blob exists. A nonce per 64 KiB chunk,
      derived (keyed) from that chunk's own plaintext and index ("synthetic
      IV" per chunk, as AES-GCM-SIV does per message), would encrypt each
      chunk as it is read: one read, still deterministic (catalogue and
      the owner's store still agree), and no nonce shared by different plaintexts.
      Costs: blob format version 2 (protocol, test vectors, a new ADR in
      place of 0003, a security review), 24 bytes more per chunk (~0.04 %),
      and equal chunks at the same index in two files show as equal (today
      only whole equal files do). Gain: the first catalogue of big files
      reads them once (about half the time on a hard disk); small files gain
      little (the page cache already serves the second read). Prefixes of
      the first X bytes, or other samples, are not enough: two files that
      differ only where the sample does not look would share a nonce.
- [ ] **Parallel catalogue:** one file at a time today; measure two on an
      SSD and on the HDD before changing it.

## Upstream

- **iroh-blobs: a store on a runtime it is given.** Each `FsStore` builds
  its own tokio runtime, so every reader batch starts and stops a set of
  threads (memory, see `threads.rs`). Since ADR 0017 the owner opens no
  store per suitcase, so this matters only for the reader's batches. Draft
  issue/PR in `docs/upstream/iroh-blobs-shared-runtime.md`, to be sent by
  Giorgos. If accepted: pass our runtime to batches; drop the
  `TOKIO_WORKER_THREADS` setting.

## ADR 0008, from the first manual test (2026-10-01)

- **Show reading progress per linked entry.** A big linked folder shows
  "reading…" until its first files are recorded, then a growing size; on a
  slow disk that looked stuck. Show "reading 40 %" (files or bytes done of
  the walk) from the pass's progress events.
- **Linking again should not read again.** Removing a linked entry forgets
  its files' records (so a reader never gets a stale entry), and adding the
  same folder back reads all of it again (104 GB on a slow disk, twice in the
  test). Keep the records of a removed entry for a while, keyed by its path,
  and reuse them if the same path is linked again with unchanged stamps.
- **macOS asks for the Downloads folder** ("OurStick requests access to
  files in the Downloads folder", 2026-10-08), for sticks received there,
  with macOS's generic text (our own, `NSDownloadsFolderUsageDescription`,
  is in `src-tauri/Info.plist` since ADR 0020); ask only when a stick is
  first received there, not at start.
- **Windows: the drive marker `.ourstick-drive` shows in Explorer** (a dot
  does not hide a file there). A user may delete it, and the stick then
  counts as ejected. Mark it hidden and system (`attrib +h +s`) when it is
  made, and on start for existing ones.
- **Third-party licences:** done (2026-10-09). `THIRD-PARTY-LICENSES.html`
  at the root, shipped in every package (`bundle.resources`); made again
  with `tools/licenses/make.sh` whenever the dependencies change. Still to
  do: a link to it from the app (an About line).
- **The GUI mockup shows one stick.** `docs/gui-mockup.html` and its image
  predate the stick list and the `⋯` menu (step 7a-iii); redraw them once
  the manual round has settled the layout.
- **Receiver (step 7b), left open after three review rounds (minor):**
  - One mutex holds every received stick's changes: stopping one (up to its
    10 s grace) makes Pause, Resume or Forget of another wait. A mutex per
    stick would not.
  - No unit tests yet for the receiver's pause/resume order, a start during
    Quit or a forget that fails: `Receivers` needs an `AppHandle`; a seam
    (a trait for the app side) would make them testable.
  - `forget_received(.., &Lock)` documents the lock it needs but cannot tell
    it is that stick's lock; a `DownloadLock { stick_id }` type would.
  - `get --follow` still has its own loop; move it onto `download::receive`
    once the window has proved it.
- **Windows VM disk:** a full debug build of the workspace with its tests
  takes about 75 GB in `target\debug`; it filled the VM's disk once
  (`LNK1180`). Clean it now and then.
- **Windows: the stick's drive letter is named after the disk it points
  into** ("Windows (O:)"); the stick's own name is in File Explorer's pane
  (ADR 0023). Only a real volume (a VHD) would name the letter itself.
- **Error words, left minor (reviews of 2026-10-02):** `words::kind` finds
  the store's errors inside LifeError, ShareError, ReceivedError and
  DeviceError, but not inside DownloadError, SpaceError, LockError,
  PlaceError or LinkedError: a database failure through those shows its own
  text. `told_one` (the watcher's error) looks at the one error, not its
  chain.
- **Linux: Quit from the dock does not quit** (it only closes the windows,
  and OurStick stays in the tray, as with ×). Tried on 2026-10-02 and undone:
  the Ubuntu Dock's Quit closes every window itself and never asks the app;
  GNOME's own Quit (Show Applications → Quit) does call a GTK `quit` action
  (`enableGTKAppId` plus a `SimpleAction` on tao's application worked), but
  without an activation token, so the Quit dialog opened behind the other
  windows. Not worth the app id's cost (the `.desktop` file must then be
  named `com.ourstick.OurStick.desktop`).
- **Windows: Quit's dialog flashed and closed once** (2026-10-02, build
  85db938, the VM): tray → Quit showed "Quit OurStick?" for a split second,
  then it was gone and the app kept running. Not seen again with 634b568
  (started with a log); cause unknown. If it comes back, start the app
  with `RUST_LOG=debug` and stderr to a file (a scheduled task with `/it`
  runs it on the VM's desktop).
- **Before release: end every test link.** Sharing links of the test
  sticks were seen in screenshots and chats during the manual tests. Remove
  the test sticks on Linux, the Mac and the VM (each one's key goes with it,
  so its links stop for good), or start from a clean install (database and
  OurStick's keyring entries deleted).
- **A scale test (asked 2026-10-02):** a stick of 100 000 small files, 20
  folders deep, Linux → Windows (and → Mac): time and memory on both sides,
  the window's responsiveness, and paths over Windows' 260 characters. Today
  there is no limit on size, number of files or depth, but none of this was
  measured at that size.
  - **First part done 2026-10-03, CLI, Linux → the same Linux** (made-up
    files of 1–8 KB, 464 MB, 224 257 folders, 20 levels; NVMe): every file
    came, identical (`diff -r`). The catalogue: 8 s. The first `get`: 17:51,
    about 93 files a second (433 KB/s), CPU about a quarter of the time on
    both sides, so both mostly wait. A second `get`, nothing changed: 3 s,
    the manifest 14.6 MB. Memory at most: sender 1.24 GB, reader 794 MB
    (366 MB for the second `get`).
  - **Small files are slow (to measure, then fix):** each file is its own
    request, one after another (`fetch_files` → `fetch_blob`), then a
    `sync_all`, a rename and a store commit, each in turn. Measure where the
    10 ms a file go (timing logs; `strace` is not allowed here), then
    likely: fetch several blobs of a batch at once, and sync and record per
    batch rather than per file. Done: ADR 0014 (placing and records) and
    ADR 0016 (fetching).
  - **Memory:** 1.24 GB for the sender of 100 000 files is too much
    (Rescued_Disk has 146 297, a million is not absurd). Find what holds it (the
    catalogue's `Vec`, the manifest kept whole, the suitcase plans; the
    owner's part is ADR 0017's, measured again after it).
  - **Watching:** the 224 257 folders went past Linux's watch limit
    (`max_user_watches` 65 536 here); the sender said so and checked every
    60 s instead, as designed. Every 60 s it walks the whole folder: measure
    what that costs on Rescued_Disk (a spinning disk), and whether to say how to
    raise the limit.
  - Still to do: Linux → Windows and → Mac with the app (the window's
    responsiveness, paths over 260 characters).
- **The manifest is sent whole at every new version** (about 100 bytes a
  file: 100 MB for a million files, at each change). If the scale test shows
  it matters: send only what changed since the reader's version (a protocol
  change; its own ADR).
- ~~**Who has it**~~ (ADR 0015, done 2026-10-04): tried by hand, Linux →
  Mac and Linux → Windows VM at once: both downloading with their percent
  (up to 30 s behind, as reports go), then "A Mac has all of it, up to
  date.", "A Windows PC has what it chose, up to date." (it left a folder
  out) and "Everyone who received it is up to date."; each line went at
  its computer's Forget. An empty list now shows nothing (the user's
  call: "No one on this list yet." made one wonder what the list is).
- ~~**Choosing inside one big folder**~~ (from the big-folder test, 2026-10-03):
  done as ADR 0012, Amendment 1 (a top-level folder opens; paths of two
  parts left out). Tried by hand 2026-10-03, Linux → Mac and Linux →
  Windows VM: a one-folder stick shown open, `MUSIC` inside it left out
  ("Not getting: MUSIC (inside Disk)") while files added later inside it
  came, then ticked again and got.
- **The big-folder test, what it showed so far (2026-10-03):** a VM whose disk
  is a file on the sender's own disk made the sender give way to its own
  reader (ADR 0013 doing its job, in a test layout no two real computers
  have); with the VM's disks on another drive the sender read at 115-143
  MB/s and the VM received about 45 MB/s. A file name with U+0080 was saved
  under another name (ADR 0010), as it should.
- **Next, in this order (agreed 2026-10-02):**
  1. ~~ADR 0012, choosing what to get~~ (done 2026-10-03, tried Linux → Mac:
     a 104 GB folder left out, a file un-ticked kept and no longer updated).
  2. ~~ADR 0014, receiving many small files~~ (done 2026-10-03; measured
     in the ADR: Linux 5.8× and 3×, Windows VM 3.6×, the Mac 1.17×). It
     leaves two things for small files: ~~**fetching several blobs at
     once**~~ (ADR 0016, 2026-10-04: 32 at once, 264 s → 60 s for 10 000
     files from Linux to the Mac), and on the Mac **`F_BARRIERFSYNC`**
     instead of `F_FULLFSYNC` (an `unsafe` `fcntl`: Giorgos decides; it is
     most of what is left there now).
  3. ~~ADR 0013, giving way to the user's disk~~ (done 2026-10-03; on the
     spinning disk a 5 GB copy beside OurStick took 1.06-1.15× its time
     alone, 1.7× before). Not on Windows or the Mac yet (no spinning disk
     there to try).
  4. Send `Rescued_Disk` (980 GB, 146 297 files, 10 levels deep,
     no name Windows refuses, 1 579 paths over 260 characters in a Windows
     Downloads folder) as the first real scale test.
