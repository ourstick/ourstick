# ADR 0027: Android and iOS

**Status:** accepted — 2026-10-26. Reviewed by Fable, Sonnet and an
independent Opus; their findings are folded in.

## Context

A link is most often opened on a phone. Today the link page says "On a
phone? OurStick is only for computers for now." CLAUDE.md already places
the phone apps in v1: the same Tauri app and the same `ourstick-core`,
receiving first; sending from a phone only while the app is open, said
plainly, because the systems stop apps in the background.

What stands in the way today (read from the code, 2026-10-26):

- The app is a binary only (`main.rs`, `[[bin]]`); Tauri's mobile targets
  need a library with `#[cfg_attr(mobile, tauri::mobile_entry_point)]`.
- Desktop pieces that do not compile for a phone: the single-instance
  plugin, the tray and its menus, the dialog windows (`dialog.rs`,
  `about.rs`: sizes, minimize, always on top), the folder pickers, and the
  login entry's commands, called from `main.rs` without a gate.
- Desktop pieces that compile but mean nothing there: the stick as a drive
  (`drive/`), the sleep guard, "still open", the macOS keys move and Local
  Network step.
- **No secret store.** `system_keyring_backend()` returns `None` on both
  systems; the keyring module and its crates exist only for the three
  desktops. The key file works but asks for a passphrase. Rule: no silent
  fall back to a plain file.
- The app finds its data folder with `device::data_dir()` (`state.rs`),
  which knows only `OURSTICK_DATA_DIR`, `%LOCALAPPDATA%`, XDG and `$HOME`;
  the default received folder is `$HOME/Downloads/<name>`
  (`receiving/preview.rs`).
- Each received stick runs its own iroh endpoint with a follow loop
  (`receiving/mod.rs`, ADR 0009), kept up for as long as the app runs.
- The receive view reads the clipboard by itself
  (`receiving/clipboard.rs`).
- A report's `kind` (ADR 0015) is a postcard enum `MacOs | Windows |
  Linux | Other` (`report/mod.rs`, `protocol.md`); `ComputerKind::this()`
  already answers `Other` on a phone.
- The site has no `/.well-known/`, so a link never opens an app.

What the systems allow (official documentation, checked 2026-10-26):

- **Tauri 2.12** builds Android (default min API 24) and iOS (default
  15.0, built only on a Mac). clipboard-manager, opener, dialog and
  deep-link work on both; single-instance and the tray do not; the dialog
  has no folder picker there, and the opener opens URLs, not files. What
  Rust cannot reach is a small Kotlin or Swift plugin called with
  `run_mobile_plugin`.
- **Links.** Android App Links (`/.well-known/assetlinks.json` with the
  SHA-256 of the key that signs the installed app: Play's app signing
  key, known only after the first upload) and iOS Universal Links
  (`apple-app-site-association`, `applinks:ourstick.com`, the Associated
  Domains capability on the App ID), both served as `application/json`
  with no redirect. The deep-link plugin hands Android
  `intent.data.toString()` and iOS `webpageURL`, both with the fragment;
  no document says so, so it is checked on a device first. **A link opens
  the app only from another app** (a chat, mail): a tap on a page of the
  same domain stays in the browser, and a page cannot tell whether the app
  is installed.
- **Files.** iOS: the app's `Documents` folder shows in the Files app with
  `UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace`; plain
  file paths work there. Android: app-specific folders need no permission
  but are hidden and go with an uninstall; the shared `Download` folder is
  `MediaStore.Downloads`. The documentation promises plain-path access
  only for media and the app's own files; files left by an earlier
  install belong to no app; random writes by path can be slower.
- **Background.** Android 14+: a user-initiated data transfer job
  (`setUserInitiated`, a notification, started while the app is visible;
  Android 13+ asks `POST_NOTIFICATIONS` at run time). A `dataSync`
  foreground service is limited to 6 hours a day from Android 15 and must
  be declared in Play with a video. iOS 26+: `BGContinuedProcessingTask`,
  started by a tap, runs on with a progress the system shows and the user
  can cancel; without it an app gets "a limited amount of time".
- **iroh 1.2** supports both. On Android its DNS needs the NDK context,
  which Tao provides, and nothing tells it the network changed (netwatch's
  Android monitor is empty): the app must call `Endpoint::network_change()`.
  The relay's TLS uses `webpki-roots`. Cellular networks often need the
  relay. The port mapper (a default feature) searches with SSDP
  multicast, which iOS allows only with a multicast entitlement; direct
  LAN paths raise iOS's local network prompt.
- **Secrets.** iOS: `apple-native-keyring-store`'s `protected` store (the
  data protection keychain, `SecretBackend::ProtectedKeychain` of ADR
  0021) supports iOS; its items have outlived an uninstall so far, which
  Apple says not to rely on. Android: `android-native-keyring-store` 1.0
  (keyring-core, values encrypted by an Android Keystore key, needs the
  NDK context Tao sets).
- **Stores.** Play: target API 36 for new apps and updates from 31 August
  2026 (edge to edge with no opt out, predictive back); a personal account
  made after 13 November 2023 needs 12 testers for 14 days of closed
  testing before production; the data safety form (end-to-end encrypted
  data no one else can read is not "collected"). App Store: guideline
  5.2.3, a privacy policy and the App Privacy answers, export compliance
  (standard encryption not from the system; the open-source exemption
  needs the source public), a working link for the reviewer. One Apple
  Developer Program membership covers iOS, TestFlight and the Mac.

## Decision

1. **One app, gated, not forked.** The Tauri app becomes a library
   (`lib.rs` with `run()`, `crate-type = ["staticlib", "cdylib", "rlib"]`)
   and `main.rs` calls it. Every desktop piece above moves behind
   `#[cfg(desktop)]`. Each command the window calls exists on every
   system; where a phone has no such thing it answers plainly, never
   silently. A desktop build does not change.
2. **Receiving first, Android first.** The first phone app only receives:
   open a link, see what is in it, choose, get it, open the folder.
   iOS follows with the same code. Sending from a phone is its own ADR.
3. **Where files go.**
   - iOS: `Documents/<name>`, shown in the Files app under "On My
     iPhone › OurStick". Plain file paths, as on a desktop.
   - Android: `Download/OurStick/<name>`, written with plain file paths,
     **if** the first device check shows that Android 11 and the newest
     let the app create, resume, rename and set the time (`set_modified`
     is fatal in the core) of any kind of file there. A folder of that
     name left by an earlier install is not the app's: a new name, as for
     any folder that is not ours (`<name> (2)`); the user removes the old
     one in the Files app. The device check passed on Android 16 (see
     "Device check" below). If the check fails, the app's own
     external folder, and the view says where it is and how to reach it;
     no "Export to…" that would keep a second full copy (ADR 0007).
   - Minimum: Android 11 (API 30), iOS 15 (Tauri's).
   - No folder choice and no "Move" on a phone at first: one place, said
     in the stick's view. "Open" opens the system's Downloads (Android)
     or the Files app at the folder (iOS), through the plugin.
4. **Keys in the system's store, never a plain file.**
   - iOS: `SecretBackend::ProtectedKeychain`, the same name for the same
     store as on macOS, with `ThisDeviceOnly` accessibility (never in
     iCloud); ADR 0021's move from the login keychain stays macOS only.
     `apple-native-keyring-store` becomes a dependency for iOS too
     (approved): its `protected` store supports iOS. The access group
     stays the Mac's (`<team>.com.ourstick.OurStick`, iOS's default); on
     the simulator, which does not sign, the store opens without it.
   - Android: a new `SecretBackend::AndroidKeystore` through
     `android-native-keyring-store` (approved 2026-10-26 with Fable's
     conditions): default features, no `user_auth_timeout`, so a changed
     screen lock does not lose the key; only a release with its fix for a
     lost Keystore key read as corrupt entries (PR #27, merged 2026-10-03,
     after 1.0.0), or that commit pinned by `rev` with Giorgos's yes. Its
     first test: write, kill, update the app, read; change the screen
     lock, read. Its name is stored in `device.secret_backend`: a
     migration widens the check, and `schema.md` says so.
   - With neither available, the app says it cannot keep keys on this
     phone and does not receive (no silent fall back).
   - iOS keeps keychain items after an uninstall, and they hold tickets.
     A first start with no database deletes this app's items, then starts
     clean. This is allowed only because a phone has never shared a
     ticket (rule 9 protects senders' keys); sending's ADR must revisit
     it. Lost with them: the reader key, so the sender sees the phone as
     a new receiver.
   - Backups: the data folder is kept out (Android `allowBackup=false`
     and data extraction rules; iOS `isExcludedFromBackup`), since its
     database would come back without its keys. Received files are the
     user's own and follow the system's backup choice.
5. **The data folder comes from the app.** On a phone `state.rs` takes
   Tauri's app data folder (iOS Application Support, Android `filesDir`)
   instead of `device::data_dir()`, and passes it on as today; the core
   does not change, and no global holds it. It stays a private folder,
   never an app group (iOS kills a suspended app holding a lock in a
   shared container). Default file protection on iOS, so a locked phone
   can still write.
6. **Links.**
   - `ourstick.com/.well-known/assetlinks.json` and
     `apple-app-site-association` (`/s/*`), served as `application/json`
     by `_headers`, with no redirect and no change to the CSP of `/s/`.
     First with the upload key's fingerprint, then Play's app signing
     key's as soon as it exists.
   - A link opened from a chat or mail opens the app, which hands it to
     the same `preview_link` the desktop paste uses.
   - A link page already open in a phone's browser shows "Copy the link"
     (as today), then "Open OurStick" (Android: an intent naming only the
     package, no data; iOS: a Smart App Banner without an argument), and
     the app offers "Paste the link". The store button when the app is
     not installed. Still no scheme or argument that carries a key (ADR
     0022).
   - The clipboard is read only when the user taps "Paste the link" (iOS
     asks each time it is read; Android shows a notice).
   - Checked before the first test build ships: the link is never logged
     on its way into the app, and a malformed link does not crash it.
   - Android: a link arrives at a small activity of its own
     (`excludeFromRecents`, `noHistory`) that hands it to the core and
     starts the main activity without it, so the system's recent tasks
     never keep the link as the main task's intent. The system log at
     launch cannot be avoided (see "Device check"); only the system and
     `adb` read it.
7. **Receiving on a phone is one stick at a time, and only while asked.**
   - A received stick's endpoint runs only during a "Get" the user
     started; one at a time, the others queued. No follow loop and no
     waiting (ADR 0025) in the background: when the app opens, each
     received stick is checked once for a new version, and its view says
     when it was last checked.
   - First release: only while the app is open, with the screen kept on
     during a "Get", and the words "Keep OurStick open until it
     finishes; it goes on from where it stopped when you open it again."
     Resume exists already.
   - Next (step 6): Android 14+ a user-initiated data transfer job with a
     progress notification and "Pause", asking `POST_NOTIFICATIONS` at
     "Get"; Android 11–13 stays "while open". iOS 26+ (step 8):
     `BGContinuedProcessingTask` started by "Get". No `dataSync` service.
   - Receive state lives in `AppState`, never in the window: Android may
     destroy and recreate the window while the job keeps the process,
     and setup must not run twice in one process (its locks would refuse
     themselves). Checked on the device.
8. **Network.** The app tells the core when the network changes (Android
   `ConnectivityManager`, iOS `NWPathMonitor`) and the core calls
   `network_change()` on its endpoint: a new `Fetcher` method, a no-op in
   memory tests. The port mapper is off on phones, at run time
   (`PortmapperConfig::Disabled` in `transport/iroh/` when built for a
   phone; approved): its UPnP search is multicast, which iOS allows only
   with an entitlement, and iroh 1.2 cannot turn on only its unicast
   PCP and NAT-PMP. No feature change in `Cargo.toml`; should a later
   iroh offer that choice, phones get PCP and NAT-PMP back.
   iOS asks for the local network with `NSLocalNetworkUsageDescription`,
   in words like ADR 0020's; denied, public addresses or the relay, said
   as on the desktop. Fewer blob store workers on a phone (`threads.rs`).
9. **One window, sized for a phone.** The same Svelte views with a phone
   layout: safe-area insets (edge to edge), the system back gesture wired
   to the views, no desktop-only words. A build target the oldest
   supported WebView runs, tested on the oldest device at hand: Chrome 88
   or Safari 14, since Svelte 5 needs `:where` and `replaceAll`. An older
   web view (Android 11's own WebView 83, never updated) shows a note to
   update Android System WebView instead of a blank window.
   `Platform` in `api.ts` gains `android` and `ios`.
10. **Reports stay as they are.** A phone sends `kind: Other`, as
    `ComputerKind::this()` already does, so every sender's app shipped
    today reads it. "A phone" in the sender's line needs a new report
    version and is not part of this ADR.
11. **Shipping.**
    - Package and bundle id `com.ourstick.OurStick`, the desktop's, on
      both: permanent once uploaded (approved).
    - Android: Play App Signing; internal testing as early as the first
      build that receives; closed testing, which the Play account needs
      (a personal one, made in 2026: 12 testers for 14 days), started as
      early as it can be; then production.
      Data safety: nothing collected. The privacy page names the phone
      apps and the public relays (n0) that see IP addresses.
    - iOS: the Associated Domains capability on the App ID; TestFlight
      first, on the test iPhone only with its owner's yes; a sender kept
      online from the Mac during App Review; export compliance and App
      Privacy answers checked before the first upload.
    - Android builds on Giorgos's Linux PC, which has the SDK, the NDK
      and the test phone (allowed there since 2026-10-26); iOS builds on
      the Mac (`giorgos` account). CI checks the Android target with the NDK that
      GitHub's runners carry (rusqlite, ring and blake3 compile C even
      for `cargo check`).

## Order of work

Each step ends with the desktop checks green; from step 0 on, real
devices over paper (CLAUDE.md).

0. **Device check, no repo change.** A scratch Tauri app on the test
   phone (a Redmi Note 15 Pro 5G, HyperOS; Android 11 on an emulator), with the core and the iroh-blobs fork built by the NDK, an
   in-memory secret store (debug only, never shipped), and a dummy
   fragment, never a real ticket:
   - a link from a chat reaches the app with its fragment whole
     (verified with `adb shell pm set-app-links`, no site deploy);
   - create, resume, rename, `set_modified` in `Download/OurStick/` on
     Android 11 and the newest, and after a reinstall;
   - a transfer from the Mac over Wi-Fi (direct?) and cellular (relay?);
   - the window recreated while a transfer runs.
   It settles decision 3 and tells whether the rest holds. Done on the
   test phone and on Android 11 on 2026-10-26 (see "Device check").
1. The app as a library, desktop pieces behind `cfg(desktop)`; no
   behaviour change on the desktop. CI: the Android check.
2. Core and app for a phone: the Android keystore backend and its
   migration, iOS on `ProtectedKeychain`, the data folder from the app,
   `Fetcher::network_change`, one receive at a time, the clipboard on tap,
   the port mapper off.
3. The phone layout of the receive path; "Open" through the plugin.
4. Play internal testing (the first upload gives Play's signing key);
   the `.well-known` files and the link page for phones (site deploy with
   Giorgos's yes); closed testing starts now (12 testers, 14 days).
5. Play production.
6. The Android transfer job, its notification and the network callback.
7. iOS on the simulator: build, keychain, `Documents`, layout.
8. iOS on a device (with its owner's yes): Universal Links, the local
   network words, `BGContinuedProcessingTask`; TestFlight, then the App
   Store.

## Device check (step 0)

2026-10-26, a scratch app outside the repo (Tauri 2.12, the deep-link
plugin, `ourstick-core` and the iroh-blobs fork built by NDK 28) on the
test phone (HyperOS, Android 16, API 36), a test stick shared by the CLI
from a Linux PC on the same network. Neither the core nor the CLI needed
a change to build for Android.

- **Files in `Download/OurStick/`**, plain paths, no permission asked:
  create, write, reopen and write on (resume), rename, `set_modified`
  (read back exactly), folders inside folders. All passed.
- **After an uninstall and a new install**, files the earlier install
  wrote can no longer be read, written, renamed, removed or given a time,
  and its folders cannot be renamed; a rename onto one of its files
  fails. New files can be written inside its folders. Hence the new name
  of decision 3. The same on Android 11 (API 30, the emulator), except
  that there the earlier install's folders can be renamed.
- **Android 11's WebView** draws the page under the status bar and gives
  `env(safe-area-inset-top)` as 0: step 3 takes the insets from the
  system, not from CSS alone.
- **A download** of 453 MB in 209 files: direct within a second on
  Wi-Fi, every file identical (md5) and with its time. Killed halfway
  and started again, it went on and ended identical. On 5G it went
  direct too (IPv6), at about 10 MB/s. Turning the phone while it ran
  did not stop it.
- **A link** opened by another app (`am start`, the app link approved
  with `pm set-app-links`) reached the app with its fragment whole, with
  the app closed and with it open. A link tapped by Giorgos and opened
  in Firefox: Firefox offered the app or itself, and through the app the
  fragment arrived whole.
- **The link in the system's records:** HyperOS logs the whole intent,
  fragment included, to the system log when the app opens
  (`SoScStageCoordinator`, `RecentsModel`, `MiuiFreeformModeController`),
  and the recent task keeps it as its base intent. Apps cannot read
  either; `adb` can. Hence the link activity of decision 6.
- **In the background** with the screen locked, the download stopped at
  once and the connection was lost about 35 s later, while the process
  lived on. As decision 7 expects: the first release works while open.
- To look at in step 2: about 6 s between large files while the received
  file is written into `Download/`, and about 30 s to check 200 MB
  already there before a resumed download goes on.

## Not decided here

- Sending from a phone (its own ADR, after receiving works).
- "A phone" in the sender's line (a new report version).
- A phone receiving straight into another app (Photos, a share sheet).
- `synchronous = NORMAL` for SQLite on phones; FULL stays (rule 9).
- Our own relay (ADR 0006), even if cellular leans on the relays more.

## Consequences

- `ourstick-core` gains one secret backend (with a migration) and a
  network-change call; the protocol does not change.
- The app's desktop code is gated, not forked.
- Approved by Giorgos on 2026-10-26, after Fable's review:
  `android-native-keyring-store` (with the conditions of decision 4),
  `apple-native-keyring-store` for iOS, the port mapper off on phones at
  run time, and the package id `com.ourstick.OurStick`.
- The link page stops saying phones are not supported only when the
  Android app is in production.
