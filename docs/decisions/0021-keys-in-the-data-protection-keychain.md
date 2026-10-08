# ADR 0021: On macOS, the keys in the data protection keychain

**Status:** accepted — 2026-10-08. Reviewed by two other models against
our code and Apple's rules; this version takes in their changes. Giorgos
approved the `protected` feature, and rule 7.

## Context

On macOS the app and the CLI keep every secret (each stick's signing seed,
stick key and node key, each received stick's link and reader key, and the
device's older node key) as generic passwords in the user's login keychain,
the "legacy" file-based keychain (`apple-native-keyring-store`, feature
`keychain`; `device.secret_backend = keychain`). Its access list names the
program that made an item; any other program, and the same one signed
differently, is asked about, once per item. Seen on 2026-10-08 with the
first build signed with the Developer ID: 14 questions in a row for 14
items, the user's password for each. A user who clicks "Deny" leaves the app
unable to share or receive.

The data protection keychain (macOS 10.15 and later, every Mac) has no access
lists and never asks: an item belongs to a keychain access group, which only
a program signed by our team with that group in its entitlements can read.
`apple-native-keyring-store` has it as its `protected` store. What it needs:

- The app signed with three entitlements:
  `com.apple.application-identifier` = `HHL7R3Q3XB.com.ourstick.OurStick`,
  `com.apple.developer.team-identifier` = `HHL7R3Q3XB`, and
  `keychain-access-groups` = [`HHL7R3Q3XB.com.ourstick.OurStick`]. All three
  are restricted: outside the Mac App Store they need a **Developer ID
  provisioning profile** for that App ID, embedded in the bundle
  (`Contents/embedded.provisionprofile`). An app that claims them without a
  matching profile is killed at launch; one without them is refused every
  item (`errSecMissingEntitlement`).
- The `protected` feature of `apple-native-keyring-store`, beside `keychain`
  (still needed to read the old items): a change to the root `Cargo.toml`,
  which needs Giorgos's approval.
- The CLI cannot carry a profile. It shares the app's data folder and
  `device` row, and on macOS it uses the login keychain too today.

Rule 9 of CLAUDE.md: a key is never made again once a link was shared. The
keys already in the login keychain must be moved, not replaced.

## Decision

1. **The data protection keychain on macOS, for the app.** In the app's
   access group, set explicitly (the store's `access-group` modifier), never
   synchronized with iCloud, with the accessibility
   `AfterFirstUnlockThisDeviceOnly`, set explicitly: readable once the user
   has logged in since the Mac started (the app shares in the background and
   may start at login), and never copied to another Mac by a backup or
   Migration Assistant, as on Windows (`CRED_PERSIST_LOCAL_MACHINE`): a
   stick's keys on two computers would be two owners of one stick. So a new
   Mac does not take over the old one's sticks; it shares them anew (new
   links). A read refused because the Mac was not yet unlocked is an error
   that says so and is tried again, never "missing", and never a reason to
   make a key. `device.secret_backend` gains `protected-keychain` (a
   migration, so no older app opens the database afterwards:
   `NewerDatabase`; `docs/schema.md`).
2. **The device row decides which store opens**, in one place
   (`device::open_secrets(row)`), which every caller uses instead of the
   platform's default (`open_system_keyring` in five places of the app
   today). A row that says `keychain` opens the login keychain until the
   move; `key-file` is never moved.
3. **The move, once, in the app only**, at the start of a version with this
   ADR, under `locks::setup_lock`, before any stick is shared or received,
   any watcher runs or any command touches a key:
   - It first says what will happen: "macOS will ask for your password once
     for each of OurStick's keys (N), one last time." `Continue`.
   - The items: for each owned stick, its signing seed and stick key, and
     its node key if it has its own (ADR 0009); the device's node key if
     `device.node_id` is set; for each received stick, its link, and its
     reader key if it has one (it may not). Any other item absent is an
     error (schema.md), never skipped.
   - For each: read the old item; if the new store already has it, equal,
     skip; different, stop and name the stick (never overwrite, never pick
     one); absent, write it, read it back and compare. In memory only;
     buffers zeroized; values never logged (stick ids may be).
   - All there: record `protected-keychain`. The old items are deleted at a
     later start, once a whole start has read every key from the new store;
     that deletion runs at every start while any remain, and asks nothing
     more than macOS asks for it (to be counted on the Mac).
   - A refusal ("Deny") or any failure: everything stays as it was (both
     copies, the old backend recorded, the app working on the login
     keychain), it says why, and offers "Try again"; it is not tried again
     by itself at every start.
4. **The CLI on macOS** cannot read the data protection keychain. On a
   device whose row says `protected-keychain` it says: "This Mac's keys are
   kept by the OurStick app. Share from the app, or use the CLI with its
   own data folder (OURSTICK_DATA_DIR) and --key-file." Before the move it
   goes on with the login keychain as today. The move is the app's: the CLI
   never starts it.
5. **Builds without the profile.** A debug build (`cfg(debug_assertions)`,
   `cargo tauri dev`) keeps the login keychain for a device it makes, at
   compile time, never by a setting; on a device already moved it refuses
   with the message above and needs its own `OURSTICK_DATA_DIR`. A release
   build without the entitlements is refused at every key with that reason
   (no fall back, CLAUDE.md).
6. **Building.** `src-tauri/OurStick.entitlements` lists the three
   entitlements literally; `tauri.conf.json` names it
   (`bundle.macOS.entitlements`), puts the profile at
   `Contents/embedded.provisionprofile` (`bundle.macOS.files`, from
   `OURSTICK_PROVISIONING_PROFILE`), and raises `minimumSystemVersion` to
   10.15. `make-dmg.sh` refuses, before notarizing, a build whose profile is
   missing or expired, or whose signature (`codesign -d --entitlements :-`)
   lacks the access group. The profile is not in the repo (it names the
   team and the certificate).

7. **Keys stay on the computer that made them, on every system.** A
   stick's keys on two computers would be two owners of one stick: the
   same identity on the network, each serving its own files. So no key
   moves by itself, owner's or reader's, on any system: macOS by
   `ThisDeviceOnly` (rule 1), Windows by `CRED_PERSIST_LOCAL_MACHINE`
   (already), Linux as far as its keyring allows (the Secret Service's file
   is copied with a home folder; noticing that the computer changed, and
   asking before sharing, is a later step). Moving a stick to a new computer
   becomes an explicit action of the app ("Move to another computer"),
   which stops it on the old one: a later ADR. The same rule for received
   sticks keeps it one rule: a new computer adds their links again.

## What Giorgos does, once

In the Apple Developer account (Certificates, Identifiers & Profiles):

1. **Identifiers → +** → App IDs → App → description "OurStick", explicit
   bundle ID `com.ourstick.OurStick`; no capability to tick.
2. **Profiles → +** → Distribution → **Developer ID** → that App ID → the
   Developer ID Application certificate → download the
   `.provisionprofile`.
3. Put it at `~/ourstick-profiles/OurStick.provisionprofile` on the Mac.
   It expires (its date is in the profile): `make-dmg.sh` says so before
   it does; a renewed certificate needs a new profile. The keys stay: the
   access group is the team's.

## To verify on the Mac first

- ~~A throwaway build: whether a team-prefixed application group works as
  the access group without a profile.~~ Tried on 2026-10-08 (macOS 26.7,
  a binary signed with the Developer ID, the hardened runtime and
  `com.apple.security.application-groups` =
  [`HHL7R3Q3XB.com.ourstick.OurStick`]): every write, read and delete in
  that group refused with `errSecMissingEntitlement` (-34018), as unsigned.
  The profile is needed.
- That the signed app with the profile starts, reads and writes, and asks
  nothing; that an item survives an update signed the same way; that it
  works on an Intel Mac too.
- How many times a whole move asks, the deletion of the old items included;
  "Deny" on one of them.
- That every link still works after the move, and after the deletion.

## Files that change

`secrets/keyring.rs` (the protected store), `device.rs` (`open_secrets`),
`store/types.rs` (`SecretBackend::ProtectedKeychain`), a migration,
`docs/schema.md`, `docs/protocol.md` ("Key storage"), the app's callers of
`open_system_keyring`, the CLI's `secrets.rs`, `tauri.conf.json`,
`OurStick.entitlements`, `make-dmg.sh`, the root `Cargo.toml` (approval).

## Tests

The move over two fake stores: every item copied and checked before the
backend is recorded; equal skipped, different stopped, absent reader key
skipped, any other absence an error; a failure (or a refusal) on the Nth
item leaving both and the old backend; a second run after a failure
finishing; a run after recording deleting the old items only after a start
read them all from the new store; no key made at any step; two programs
(the setup lock). The CLI's message on a moved device. The backend in the
store (migration). The rest by hand on the Mac.

## Alternatives considered

- **One login-keychain item per stick.** Fewer questions, still questions
  after every update signed differently, and a format to migrate anyway.
- **"Always Allow" and nothing else.** It holds only while the signature
  stays the same; the first signed build already asked 14 times.
- **A key file for the app.** A passphrase at every start.
- **An application group as the access group, no profile.** Simpler if it
  works; to be tried first (above).
