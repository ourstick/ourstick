# To do (not code)

Things that are waiting on something other than code. Code work belongs in
the implementation order in `CLAUDE.md`; limits of the code are in
`docs/protocol.md` ("Known limits") and `docs/gui-spec.md`.

## GitHub

- [ ] Make the GitHub organisation (`ourstick`, as in `repository` in
      `Cargo.toml`) and push the repository there. Today it lives only on the
      private `hetzner` remote.

## Upstream

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
- [ ] **Parallel catalogue:** one file at a time today; measure two on an
      SSD and on the HDD before changing it.
