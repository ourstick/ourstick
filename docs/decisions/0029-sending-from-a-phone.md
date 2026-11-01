# ADR 0029: Sending from a phone

**Status:** accepted — 2026-11-01. Reviewed by Fable, Sonnet and an
independent Opus; their findings are folded in. Question A was settled on the test phone
(2026-10-31, "Device check") and B and C answered by Giorgos (2026-11-01).

## Context

ADR 0027 made the phone apps receive first and left sending to its own
ADR. Giorgos looked at the Android app on his phone for the first time
(2026-10-31) and decided that sending comes **before the first Play
upload**: many people use only a phone, and an app that can only receive
looks unfinished to them, with nothing on Home to say where sending is.

What stands in the way (read from the code, 2026-10-31):

- **The core reads the sender's files by path, in many places**, and
  takes their identity from paths:
  - the manifest: `manifest/catalogue.rs` and `catalogue/walk.rs`
    (`walkdir`, `symlink_metadata`), `catalogue/known.rs`,
    `manifest/hashing.rs`, `manifest/stamp.rs` (`FileStamp`: size,
    mtime and ctime in nanoseconds; an unchanged stamp skips hashing);
  - blobs made when asked (ADR 0017): `blob/made.rs` (`MadeBlob::open`,
    `FileReads`), `share/files_keeper/make.rs`,
    `transport/iroh/made_store.rs` (a `metadata` per request) and
    `made_store/reads.rs`, `disk/turns/id.rs` (`DiskId::of_file`, st_dev);
  - places: `layout.rs` (`on_disk()` joins a root and a relative path;
    `Root::is_gone`), `linked.rs` (`plan_link`, names from
    `file_name()`, `disk_of`, `is_absolute`, no symlinks), `places.rs`
    (`comparable`, `canonicalize`, overlap and clash checks),
    `places/sending.rs` (`is_whole_disk`), `places/measure.rs` (sizes for
    the Send list), `share/mod.rs` (`plan_share`, `folder_size`);
  - the watcher (`watcher/`: stamps and the settle check).
- The app's serving code (`serving/`, `send.rs`, `sharing.rs`) compiles
  for Android; the Send view, the pickers and the clipboard copy are
  desktop-only or go through Tauri's clipboard plugin, which marks
  nothing sensitive. The phone's Home shows Receive alone.
- A phone's stick keys live in the Android Keystore or the iOS keychain,
  and the data folder is kept out of backups (ADR 0027 decision 3).
- ADR 0027's device check: with the screen locked the transfer stopped at
  once and the connection was lost about 35 s later. Sending a link means
  leaving OurStick for a chat app, and the receiver opens it later.

What the systems allow (official documentation and the reviews,
2026-10-31; facts marked *(check)* are settled in step 0):

- **Android, reading.** Plain paths reach only the app's own files and,
  with `READ_MEDIA_IMAGES`/`VIDEO`, media. Play allows those only to apps
  whose core is broad photo access when the system picker is not enough,
  and `MANAGE_EXTERNAL_STORAGE` only to file managers, backup, anti-virus,
  document management, on-device search, encryption and phone migration;
  it names "file selection where the user manually picks individual
  files" as not allowed. Without a permission:
  - the **photo picker**: at most `getPickImagesMaxLimit()` items per
    pick (often 100); location may be removed from photos; on Android
    11–12 without its backport it falls back to `ACTION_GET_CONTENT`,
    whose grants cannot be kept *(check)*;
  - the **Storage Access Framework**: `ACTION_OPEN_DOCUMENT` (files) and
    `ACTION_OPEN_DOCUMENT_TREE` (a folder and everything under it; never a
    volume's root, `Download/` itself, `Android/data` or `Android/obb`);
  - `takePersistableUriPermission` keeps a grant across restarts, at most
    512 per app (128 before Android 11);
  - a file is read through `ContentResolver.openFileDescriptor`; a
    folder is listed with one `DocumentsContract` query per folder (name,
    size, last modified in ms or null, type), with no ctime and no
    reliable change notice. Names come from the provider: `/`, the same
    name twice, names differing in case are possible. A cloud provider may
    download the whole file before it answers, or give a pipe.
  - Reopening `/proc/self/fd/<n>` runs a new `open()` as the app: on the
    test phone it was refused for everything but the photo picker's
    photos (see "Device check").
- **Android, the link.** `setRecentsScreenshotEnabled` and
  `EXTRA_IS_SENSITIVE` exist from API 33; Android 11–12 have `FLAG_SECURE`
  (which also blocks screenshots). The share sheet's own Copy (Android
  14+) marks nothing; keyboards may keep clipboard history.
- **Android, background.** A user-initiated data transfer job (Android
  14+, `RUN_USER_INITIATED_JOBS`, started while the app is visible, a
  network constraint, an estimated size, a notification with Stop) is
  free of App Standby quotas, but the system stops a job that "has run
  for longer than necessary to complete the data transfer". A `dataSync`
  foreground service is limited to 6 hours a day from Android 15, needs a
  Play declaration with a video, and Google steers away from it. HyperOS
  stops background apps hard (its own battery and autostart settings).
- **iOS.** The Files app's files and folders are plain paths while a
  security-scoped URL is accessed, with a bookmark kept.
  `PHAssetResourceManager` streams a photo's original without a copy;
  an iCloud item not on the phone must be refused like a cloud pipe.
  iOS 26+: `BGContinuedProcessingTask`.

## Decision

1. **Sending on a phone is the desktop's Send, read through the system's
   pickers, never through a broad permission.** No
   `MANAGE_EXTERNAL_STORAGE`, no `READ_MEDIA_*`, no
   `ACCESS_MEDIA_LOCATION` (a photo leaves without its location, and the
   Send view says so). The phone's Home shows Send and Receive.
2. **Three ways to choose, on Android.**
   - **Photos and videos**: the photo picker; where it is missing,
     `ACTION_OPEN_DOCUMENT` with `image/*` and `video/*` (its grants can
     be kept), never `ACTION_GET_CONTENT`.
   - **Files**: `ACTION_OPEN_DOCUMENT`, several at once.
   - **A folder**: `ACTION_OPEN_DOCUMENT_TREE`; everything under it, as
     a folder chosen on a desktop. One grant for any number of files.
   Photos and files chosen one by one are each an entry at the top of the
   stick, as files chosen on a desktop; a name an entry has already is
   numbered (`IMG_0001 (2).jpg`), since a phone cannot rename them first
   as ADR 0028 asks (amended 2026-11-01: no "Photos" or "Files" folder,
   which would have needed a record of its own). Each grant is kept at
   once; at every start the grants no stick uses are given back. The Send
   view says "For many photos, choose their folder" (DCIM/Camera can be
   chosen); counting what is left of the 512 grants waits for a device
   that comes near them.
3. **Files are sent from where they are, also on a phone.** No copy into
   the app: a phone has the least room for a second one (ADR 0028).
   - **One trait for the sender's files** (rule 2), path-keyed so that
     signatures barely change: an item is a place plus a relative path,
     never a joined path. It answers `list` (entries: name, kind, stamp),
     `stamp`, `open` (a reader by position, as `MadeBlob::read_at` reads
     today), `relation(a, b)` (same,
     contains, inside, apart: `DocumentsContract.isChildDocument` on
     Android, since two URIs cannot be compared as text) and `disk`.
     `LocalFiles` is std::fs one to one, the desktop's and the tests'.
     `on_disk()`, `Root::is_gone`, `plan_link`, `comparable`, `measure`,
     the catalogue, the made store and `files_keeper` go through it.
   - **Android's `DocumentFiles`** lives in the app. It keeps each
     place's map from relative path to document URI from its last
     listing; it takes names from `DISPLAY_NAME` and passes them through
     the core's `validate_path`; it calls `ContentResolver` over JNI
     from the core's blocking threads (Tauri's plugin is async IPC
     through the activity), with a `CancellationSignal` and a time
     limit.
   - **A file stays open in Kotlin**; Rust asks it for bytes at a
     position over JNI (`readAt(position, byte[])`, 256 KiB at a time),
     from the core's own threads attached to the JVM, which the plugin
     hands over once at start (`JNIEnv::get_java_vm`). No `unsafe` in our
     code (question A); the extra copy costs nothing a receiver could see
     (see "Device check"). A descriptor that is not a regular, seekable file, or
     a provider that does not answer in time, makes that item "not on the
     phone" ("Open it once in its app, then choose it again"); the other
     items go on. A revoked grant (Clear storage, a moved folder, an SD
     card out, the user in Settings) makes the place "Not found", with
     "Find it…" to choose it again and keep the link.
   - **Stamps:** one source per implementation. On Android, size and
     last modified (ms) from the provider; a null time or size means the
     file is hashed again on each look. `docs/protocol.md`'s known limits
     say so.
   - **Room:** sending keeps no copy, but the stick's outboards (about
     0.4 % of its size, ADR 0017) and its cache live in the app's data. A
     Send that would not leave the space ledger's reserve (ADR 0008) is
     refused before the link exists, with the size it needs and what is
     left on this phone. Receiving keeps today's check ("It does not fit:
     only … are free"), to be seen once on the phone in step 3.
   - **Disks:** `disk` is unknown on Android: one disk turn for all
     places (ADR 0013), whole-disk checks off. A phone stick's own folder
     is an empty private folder; no drive.
   - **Names** from a provider are untrusted: `/`, a name twice in one
     folder or names differing only in case are decided in the one place
     that decides names today, numbered or refused, never put twice in
     a manifest. `manifest/ignore.rs` gains Android's `.trashed-*`,
     `.pending-*`, `.thumbnails` and `.nomedia`. The view escapes names
     as it does manifest names.
   Places keep a text form in SQLite (a URI on Android), with the
   migration and `schema.md` note if the stored form needs one.
4. **No watcher on a phone.** The stick is looked at again whenever the
   app comes to the front and on "Look again", with progress ("Getting it
   ready…", cancellable) and the settle check of today; changes become a
   new version. The view says when it last looked. A `ContentObserver`
   on a tree is tried in step 0 and used only if it proves reliable.
5. **The link leaves through the system's share sheet.**
   - "Send the link" opens the share sheet with the link as text.
     "Copy the link" goes through `PhonePlugin` with `EXTRA_IS_SENSITIVE`
     (the key works before API 33). The QR is shown for someone next to
     you.
   - While a link or a QR is on screen: `setRecentsScreenshotEnabled(false)`
     on API 33+, `FLAG_SECURE` on Android 11–12.
   - The link may be ready before every file is hashed: what is hashed
     is served, the rest follows as new versions (as on the desktop).
   - To be checked in step 0: the share intent, the plugin's arguments
     and Tauri's `Logger.kt` in logcat of debug and release builds; the
     share sheet's own Copy and the keyboard's clipboard history (what
     cannot be prevented is said in the privacy page).
6. **It sends while the app is open, and says so.**
   - After the share sheet the user is in the chat app. When they come
     back, the stick's view says "Keep OurStick open until they have it",
     shows who has it (ADR 0015), keeps the screen on while someone is
     getting files and suggests a charger for a large stick. The
     receiver's waiting words say the sender's OurStick must be open.
   - The view says when it sends over mobile data (`isActiveNetworkMetered`).
   - Background sending: an Android 14+ user-initiated job, started by
     Send (or by opening the stick) while the app is visible, with a
     network constraint and the stick's size as estimate, kept while the
     stick is sent; the device check found it kept for an hour with no
     receiver and no notification permission (see "Device check").
     Notifications are never asked for. `dataSync` is not used.
     Android 11–13 stays "while open". iOS 26+:
     `BGContinuedProcessingTask`.
7. **What a phone loses, said before the first Send.** "If you uninstall
   OurStick, clear its data or move to a new phone, the links you sent
   from this phone stop working." A stick that still exists can be given
   a new link. iOS's first start with no database still deletes the
   app's keychain items (ADR 0027 decision 3): without the database
   those keys belong to no stick.
8. **Words.** The Send path of ADR 0028 with phone words: "this phone",
   no drive, no tray, no "start at login", no "Change…".
9. **iOS** (after Android): files and folders from the Files app in
   place, with bookmarks and "Find it…" when one goes stale; photos
   streamed by `PHAssetResourceManager`, no copy; iCloud items not on the
   phone refused as above.

**Questions for Giorgos:**

- **A.** Settled: no `unsafe`; bytes are read through Kotlin over JNI
  (Giorgos, 2026-10-31, after the device check).
- **B.** Approved (Giorgos, 2026-11-01): `jni` 0.21 as a direct
  dependency of the app, for Android only
  (`[target.'cfg(target_os = "android")'.dependencies]`); it is already
  in the build through Tauri and the Android keyring store.
- **C.** Answered (Giorgos, 2026-11-01): Play only when the app is
  complete and sends everything, photos, files and folders (step 4 done).

## Order of work

Each step ends with `cargo test --workspace`, clippy (also for Android)
and fmt green; from step 0 on, real devices over paper.

0. **Device check, no repo change.** A scratch app on the test phone and
   Android 11 (emulator), dummy sticks only, never a real ticket:
   - descriptors (first results below): a file on an SD card,
     over 4 GB; a Google Drive and a Google Photos cloud item (pipe, block,
     time); a video still being recorded;
   - pickers: the photo picker's limit and availability on Android 11,
     bytes read against the original (location, transcoding); which
     folders the tree picker refuses; grants after a restart, an update,
     and a revoke; the grant count;
   - a tree: listing 5,000 photos over JNI, reading and BLAKE3 of 5 GB,
     duplicate names in one folder, null times, a `ContentObserver`;
   - the link: logcat during share and copy, the share sheet's Copy, the
     keyboard's clipboard history, the recents picture;
   - background: how long serving lasts after the share sheet with the
     screen on and locked, Doze after 30 minutes, HyperOS's battery
     settings; a user-initiated job with no receiver for 10 minutes and
     1 hour.
1. Core, in three commits: (a) the trait, `LocalFiles` and stamps;
   (b) places, `linked`, `measure`, `layout` and the watcher's stamps;
   (c) the catalogue, the made store and `files_keeper`. Tests unchanged
   and green, plus a stick read through an in-memory implementation.
2. App on Android: `DocumentFiles` for photos and files, the pickers,
   grants, "not on the phone", "Find it…".
3. The Send path on a phone: Home with Send and Receive, the share
   sheet, the sensitive copy, the recents picture, the words of
   decisions 6 and 7, looking again on resume.
4. Folders: the tree picker, listings, names, "Look again".
5. Background sending, as decision 6 and step 0 say.
6. Then ADR 0027 step 4 (Play internal testing), after steps 4 and 5
   (question C; Giorgos wants background sending before Play too, if the
   device check shows Android keeps it reliably).
7. iOS sending with ADR 0027's iOS steps.

## Device check (step 0)

2026-10-31, a scratch app outside the repo (Java, and a Rust library
with `jni` 0.21 and `blake3` built by NDK 28) on the test phone (HyperOS,
Android 16), files chosen by Giorgos:

- **Reopening `/proc/self/fd/<n>`** was refused (`EACCES`) for a PDF from
  Downloads (`ACTION_OPEN_DOCUMENT`), a photo from Google Drive, and a PDF
  in a chosen folder (`ACTION_OPEN_DOCUMENT_TREE`); it worked only for
  photos from the photo picker. The provider's own descriptor was a
  regular file every time, Google Drive's included (a file it had
  already downloaded).
- **Reading through Kotlin over JNI**, from a Rust thread attached with
  the `JavaVM` the native call handed over, no `unsafe`, hashed with
  BLAKE3 as the core does:
  - a 188 MB video read the first time: 94 MB/s, of which the JNI calls
    and copies were about 0.15 s of 2 s; the rest was the phone's storage;
  - read again from memory: 690 MB/s with 256 KiB pieces, 601 with 1 MiB,
    547 with 4 MiB; BLAKE3 alone about 1,100 MB/s;
  - four photos of about 7 MB: 150–180 MB/s the first time, 670 from
    memory;
  - a 5.37 GB file of known content (each MiB begins with its index),
    copied by `adb` to `Download/` just before and so partly in memory:
    441 MB/s the first time, then 490 MB/s with 256 KiB pieces, 440 with
    1 MiB, 369 with 4 MiB; Kotlin alone, without JNI, 884 MB/s. Every
    read gave the hash computed on the PC: positions past 4 GiB are read
    right;
  - the same hash on every read. Not measured: a cold first read
    without JNI.
  The network gave 8–10 MB/s (ADR 0027), so a receiver cannot tell; a
  5 GB video takes about a minute to be ready, which the storage sets.
- **The first send from the phone** (2026-11-01, a release build of
  step 3): a test folder chosen with "A folder…" (Android asked to allow
  that one folder), counted, "Get the link", "Send the link" into a chat,
  opened by the Linux app: while OurStick was in the background (the chat
  in front) the receiver said the sender was offline; with OurStick in
  front it came, every file identical (md5). The link was nowhere in the
  phone's log. The release build had removed `Documents.kt`, which only
  JNI calls: ProGuard keeps it now.
- **In the background, a user-initiated job** (a scratch app: a job with
  a network constraint and an estimate, its thread connecting out every
  10 s, the screen off, the phone on USB power): 10 minutes with the
  notification allowed, then a whole hour with **notifications denied**,
  the network answering every time, the job never stopped, though no
  data moved and no receiver came. So the app never asks for
  notifications: the job runs without them (its notice then shows only in
  the system's task manager). Not measured yet: on battery, still and
  locked for hours (Doze cannot start while the phone charges).

## Done so far (2026-11-01)

Steps 1 (all), 2 (photos, files and folders through the same code, a
clash numbered, grants tidied at start), 3 (Home, the Send view, the
share sheet, the sensitive copy, the words of decisions 6 and 7) and the
code of 5 (the job held while anything is sent, from Android 14), after a
review by Fable and Sonnet. A phone looks again every minute instead of
watching (decision 4; "Look again" and on resume are still to come).
On the test phone (Android 16) the job ran while photos chosen one by
one were sent, and a computer got them all with the phone's screen off
and OurStick not opened again. Still to come: a long test on battery
(Doze), "Find it…", telling the user when a grant could not be
kept, and `relation` for places that overlap (two chosen places are now
compared as text, so a folder and its own subfolder are sent twice).

## Not decided here

- OurStick as a share target ("Share → OurStick" from the gallery): the
  most natural phone path, but its grants cannot be kept.
- Whole albums, or new photos as they are taken.
- Forbidding mobile data for sending.
- "A phone" in the sender's line (a new report version, ADR 0027).

## Consequences

- `ourstick-core` gains one trait for the sender's files, with the
  desktop behaving as today; the protocol does not change, its known
  limits gain Android's weaker stamps.
- The app gains a JNI module, with no `unsafe`.
- No storage permission is declared to Play; a user-initiated job would
  add `RUN_USER_INITIATED_JOBS`, a `dataSync` service a declaration and
  a video.
- Losing the app's data loses the links the phone sent, said before the
  first Send.
- ADR 0027's step 4 (Play) waits for this ADR's steps 4 and 5.
