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
- [ ] Still to try: a start at login with no window, a proxy, a captive
      portal.
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
      the app must show how to let OurStick through the firewall.
- [ ] **A big folder:** about 900 GB in 146 000 files, 10 levels deep: the
      link at once, partial versions every 3 minutes, a restart halfway,
      flat memory, two readers at different places, and paths over Windows'
      260 characters.

## Performance and polish

- **Small files:** a stick of 100 000 small files (464 MB) took 18 minutes
  the first time, Linux to Linux, before ADRs 0014 and 0016 (placing and
  fetching in batches); to be measured again, and to Windows and macOS with
  the app. The sender used 1.24 GB of memory for it then, too much.
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
- **Android:** `https://ourstick.com/.well-known/assetlinks.json`, so that
  Android opens sharing links in the app; it needs the app's signing key.
