# To do (not code)

Things that are waiting on something other than code. Code work belongs in
the implementation order in `CLAUDE.md`; limits of the code are in
`docs/protocol.md` ("Known limits") and `docs/gui-spec.md`.

## GitHub

- [ ] Make the GitHub organisation (`ourstick`, as in `repository` in
      `Cargo.toml`) and push the repository there. Today it lives only on the
      private `hetzner` remote.

## Upstream

- [ ] **iroh 1.2: a direct path lost and not found again.** Two peers on one
      LAN (192.168.1.x): the direct path carried 60 MB/s, the data paused
      about 20 s (the owner preparing a suitcase), then the connection moved
      to the relay and stayed there; a new connection went direct at once.
      Also after a reader changed networks. Worked around by connecting again
      (`docs/protocol.md`, "Relay policy"); worth a report with a small
      reproduction.
- [ ] **iroh-blobs: GC logs `error during gc run: Error::Io` after the store
      shuts down, and stops for good after any error.** A public issue; the
      draft (with a standalone reproduction) is kept outside the repository.
      OurStick no longer runs iroh-blobs' GC (ADR 0007: suitcases leave by
      removing their folder), so it is no longer affected; the report still
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
- [ ] **Packages:** `.deb` (with a proper `.desktop` file and icon), `.dmg`,
      `.msi`.
- [ ] **macOS signing,** once there is an Apple Developer account: a signed
      app keeps its keychain access across updates.
- [ ] **macOS data protection keychain** (apple-native-keyring-store's
      `protected` store), which needs the signed app's entitlements and never
      prompts.
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
      suitcases within the budget (`~/.local/share/ourstick/cache/<stick>/`
      never much above it), two readers at different places.
- [ ] **Remove the ADR 0005 caches** (`cache/<stick>/blobs/` and `staging/`)
      once suitcases are proven by hand: nothing reads them any more, and on
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
      suitcases still agree), and no nonce shared by different plaintexts.
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
- **A pass running while a link is removed** may record the removed entry's
  files once more and publish them in its version; the next pass (nudged by
  the removal) drops them. Have the pass check its layout before each part.
- **macOS asks for the keychain password once per key, again after every
  unsigned build** (seen on the Mac: "many times", for the stick's three
  keys). Sign the app with a Developer ID before the first release, so
  updates keep the keychain's permission; and consider one keychain item per
  stick (its keys together) so a new program asks once per stick.
- **Windows: Explorer calls the stick "Windows (O:)"**, the label of the
  disk that holds its folder, since a `subst` letter has none of its own.
  Give the letter the name "OurStick" per user (HKCU `...\Explorer\
  DriveIcons\<letter>\DefaultLabel`, no admin rights), set with the letter
  and removed with it.
- **Windows: the drive marker `.ourstick-drive` shows in Explorer** (a dot
  does not hide a file there). A user may delete it, and the stick then
  counts as ejected. Mark it hidden and system (`attrib +h +s`) when it is
  made, and on start for existing ones.
- **Third-party licences in the packages.** Before the first public package,
  ship a THIRD-PARTY notices file (e.g. made with `cargo about`), including
  the Unicode-3.0 licence of the ICU crates (`icu_normalizer` for stick
  names, already used by iroh through `url`/`idna`).
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
  into** ("Windows (O:)", not "OurStick (O:)"). A `subst` letter reports its
  host volume's label; both per-user `DefaultLabel` keys
  (`HKCU\Software\Classes\Applications\Explorer.exe\Drives\O` and
  `HKCU\…\Explorer\DriveIcons\O`) were tried in the VM on 2026-10-02 and
  Explorer ignored them. Only a real volume (a VHD) would carry its own name.
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
  named `com.ourstick.app.desktop`).
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
    batch rather than per file. Its own ADR if the order of writing (ADR
    0007) changes.
  - **Memory:** 1.24 GB for the sender of 100 000 files is too much
    (Rescued_Disk has 146 297, a million is not absurd). Find what holds it (the
    catalogue's `Vec`, the manifest kept whole, the suitcase plans).
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
- **Dropping onto a stick, from the big-folder test (2026-10-03):** (1) while a
  drop copies (3 GB, half a minute) only the zone's title says "Copying…":
  no progress, nothing else changes, so Giorgos dropped the folder again
  and again; (2) a drop while one copies is taken, and the same folder
  comes in again silently as "new in (2)", "new in (3)"; a real stick asks
  ("already there: replace?"); (3) "4 files · 12.3 GB" counts top-level
  entries as files (they were four folders). Also: one big linked folder
  (a whole disk shared) leaves the receiver nothing to choose (ADR 0012
  chooses at the top level only): choose inside it when it is alone.
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
     leaves two things for small files: **fetching several blobs at once**
     (across a real network each request's round trip is most of the time:
     287 s for 10 000 files from Linux to the Mac, 80 ms apart; its own
     ADR), and on the Mac **`F_BARRIERFSYNC`** instead of `F_FULLFSYNC`
     (an `unsafe` `fcntl`: Giorgos decides).
  3. ~~ADR 0013, giving way to the user's disk~~ (done 2026-10-03; on the
     spinning disk a 5 GB copy beside OurStick took 1.06-1.15× its time
     alone, 1.7× before). Not on Windows or the Mac yet (no spinning disk
     there to try).
  4. Send `Rescued_Disk` (980 GB, 146 297 files, 10 levels deep,
     no name Windows refuses, 1 579 paths over 260 characters in a Windows
     Downloads folder) as the first real scale test.
